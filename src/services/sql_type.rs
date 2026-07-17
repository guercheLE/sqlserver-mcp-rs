// SQL Server 2025 - master/msdb/sandbox combined catalog MCP server.
//
// Converts between JSON (what a tool call receives/returns) and the
// concrete Rust types tiberius's `ToSql`/`FromSql` traits require, keyed by
// the `x-sql-type` string every property in the merged OpenAPI spec carries
// (see docs/sqlserver-eda-openapi-pipeline/README.md's "OpenAPI mapping
// convention"). `x-sql-type` is the exact SQL Server type text (e.g.
// `nvarchar(256)`, `decimal(18,2)`, `datetime2(7)`) — deliberately read
// here instead of the coarse OpenAPI `type`/`format`, which collapses every
// string-like type to `string` and every numeric type to `number`.
//
// Binding uses `Numeric`/`chrono` types directly rather than approximating
// decimal/numeric/money parameters as `f64`, since SQL Server's `decimal`
// supports up to 38 digits of precision — well beyond what an `f64`
// mantissa (~15-17 significant digits) can round-trip exactly.

use tiberius::numeric::Numeric;
use tiberius::{ColumnData, ToSql};

/// Strips the `(...)` length/precision suffix, if any, and lowercases —
/// e.g. `"decimal(18,2)"` -> `"decimal"`, `"NVARCHAR(256)"` -> `"nvarchar"`.
fn base_type(x_sql_type: &str) -> String {
    x_sql_type
        .split('(')
        .next()
        .unwrap_or(x_sql_type)
        .trim()
        .to_ascii_lowercase()
}

/// Extracts `(precision, scale)` from a `decimal(p,s)`/`numeric(p,s)`
/// `x-sql-type` string, defaulting to SQL Server's own defaults
/// (`decimal` with no explicit precision/scale means `(18, 0)`) when the
/// suffix is missing or unparseable.
fn precision_scale(x_sql_type: &str) -> (u8, u8) {
    let Some(inner) = x_sql_type
        .split_once('(')
        .and_then(|(_, rest)| rest.strip_suffix(')'))
    else {
        return (18, 0);
    };
    let mut parts = inner.split(',').map(str::trim);
    let precision = parts.next().and_then(|p| p.parse().ok()).unwrap_or(18);
    let scale = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    (precision, scale)
}

/// Parses a base-10 decimal string (optionally signed, optionally
/// containing a `.`) into a `Numeric` with the given `scale`, without going
/// through a floating-point intermediate — the whole point of using
/// `Numeric` over `f64` for `decimal`/`numeric`/`money` parameters.
fn parse_numeric(text: &str, scale: u8) -> anyhow::Result<Numeric> {
    let text = text.trim();
    let (sign, text) = match text.strip_prefix('-') {
        Some(rest) => (-1i128, rest),
        None => (1i128, text.strip_prefix('+').unwrap_or(text)),
    };
    let (int_part, frac_part) = text.split_once('.').unwrap_or((text, ""));
    if frac_part.len() > scale as usize {
        anyhow::bail!(
            "value '{text}' has more fractional digits than the declared scale ({scale})"
        );
    }
    let padded_frac = format!("{frac_part:0<width$}", width = scale as usize);
    let digits = format!("{int_part}{padded_frac}");
    let unscaled: i128 = if digits.is_empty() { 0 } else { digits.parse()? };
    Ok(Numeric::new_with_scale(sign * unscaled, scale))
}

/// Renders a `Numeric` as an exact decimal string, without going through
/// `Numeric`'s own `Display`/`Debug` impl — that impl formats
/// `int_part()` and `dec_part()` independently with `write!("{}.{}", ...)`,
/// and for a negative value *both* parts carry the sign (`dec_part()` is
/// literally `value - (value / scale) * scale`, so it inherits `value`'s
/// sign), producing a doubled-sign string like `"-123.-45"` for `-123.45`
/// instead of the correct `"-123.45"` — confirmed independently by this
/// module's own `parse_numeric` round-trip test failing against it.
fn format_numeric(n: Numeric) -> String {
    let scale = n.scale() as usize;
    let value = n.value();
    let negative = value < 0;
    let digits = value.unsigned_abs().to_string();
    let body = if scale == 0 {
        digits
    } else {
        let padded = format!("{digits:0>width$}", width = scale + 1);
        let split_at = padded.len() - scale;
        format!("{}.{}", &padded[..split_at], &padded[split_at..])
    };
    if negative { format!("-{body}") } else { body }
}

/// Builds a boxed `ToSql` value for one input parameter, from its JSON
/// argument value and declared `x-sql-type`. Returns `ColumnData` boxed
/// behind `ToSql` (rather than the concrete Rust type) since `ColumnData`
/// itself already carries the SQL wire type unambiguously for every
/// variant used here, keeping this function's return type uniform across
/// every `x-sql-type` branch.
pub fn json_to_param(value: &serde_json::Value, x_sql_type: &str) -> anyhow::Result<Box<dyn ToSql>> {
    let base = base_type(x_sql_type);
    let param: Box<dyn ToSql> = match base.as_str() {
        "bit" => Box::new(value.as_bool()),
        "tinyint" => Box::new(value.as_u64().map(|n| n as u8)),
        "smallint" => Box::new(value.as_i64().map(|n| n as i16)),
        "int" => Box::new(value.as_i64().map(|n| n as i32)),
        "bigint" => Box::new(value.as_i64()),
        "real" => Box::new(value.as_f64().map(|n| n as f32)),
        "float" => Box::new(value.as_f64()),
        "decimal" | "numeric" | "money" | "smallmoney" => {
            let (_, scale) = precision_scale(x_sql_type);
            match value {
                serde_json::Value::Null => Box::new(Option::<Numeric>::None),
                serde_json::Value::String(s) => Box::new(Some(parse_numeric(s, scale)?)),
                serde_json::Value::Number(n) => Box::new(Some(parse_numeric(&n.to_string(), scale)?)),
                other => anyhow::bail!("expected a number or numeric string, got {other}"),
            }
        }
        "uniqueidentifier" => match value.as_str() {
            Some(s) => Box::new(Some(uuid::Uuid::parse_str(s)?)),
            None => Box::new(Option::<uuid::Uuid>::None),
        },
        "date" => match value.as_str() {
            Some(s) => Box::new(Some(chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")?)),
            None => Box::new(Option::<chrono::NaiveDate>::None),
        },
        "time" => match value.as_str() {
            Some(s) => Box::new(Some(parse_naive_time(s)?)),
            None => Box::new(Option::<chrono::NaiveTime>::None),
        },
        "datetime" | "datetime2" | "smalldatetime" => match value.as_str() {
            Some(s) => Box::new(Some(parse_naive_datetime(s)?)),
            None => Box::new(Option::<chrono::NaiveDateTime>::None),
        },
        "datetimeoffset" => match value.as_str() {
            Some(s) => Box::new(Some(chrono::DateTime::parse_from_rfc3339(s)?)),
            None => Box::new(Option::<chrono::DateTime<chrono::FixedOffset>>::None),
        },
        "binary" | "varbinary" | "image" | "rowversion" | "timestamp" => match value.as_str() {
            Some(s) => Box::new(Some(
                base64::Engine::decode(&base64::engine::general_purpose::STANDARD, s)
                    .map_err(|err| anyhow::anyhow!("expected base64-encoded binary: {err}"))?,
            )),
            None => Box::new(Option::<Vec<u8>>::None),
        },
        // char/varchar/nchar/nvarchar/text/ntext/xml/sysname/sql_variant and
        // anything else not enumerated above: stringify. This is the safe
        // default for the curated system-catalog surface this project
        // targets (identifiers, names, free-form text), and matches how
        // SQL Server itself implicitly converts a string literal to most
        // other scalar types when needed.
        _ => match value {
            serde_json::Value::Null => Box::new(Option::<String>::None),
            serde_json::Value::String(s) => Box::new(Some(s.clone())),
            other => Box::new(Some(other.to_string())),
        },
    };
    Ok(param)
}

fn parse_naive_time(s: &str) -> anyhow::Result<chrono::NaiveTime> {
    for format in ["%H:%M:%S%.f", "%H:%M:%S", "%H:%M"] {
        if let Ok(time) = chrono::NaiveTime::parse_from_str(s, format) {
            return Ok(time);
        }
    }
    anyhow::bail!("could not parse '{s}' as a time")
}

fn parse_naive_datetime(s: &str) -> anyhow::Result<chrono::NaiveDateTime> {
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f") {
        return Ok(dt);
    }
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Ok(dt.naive_utc());
    }
    anyhow::bail!("could not parse '{s}' as a datetime")
}

/// Converts one decoded SQL Server cell into JSON. Matches on `ColumnData`
/// directly (the value tiberius already decoded off the wire) rather than
/// re-deriving a type from `x-sql-type`, since the wire is authoritative —
/// a spec's declared type can drift from what the live engine actually
/// returns (see the pipeline README's own note on `sp_help`-style
/// conditional result sets).
pub fn column_data_to_json(data: &ColumnData<'_>) -> serde_json::Value {
    match data {
        ColumnData::U8(v) => v.map(Into::into).unwrap_or(serde_json::Value::Null),
        ColumnData::I16(v) => v.map(Into::into).unwrap_or(serde_json::Value::Null),
        ColumnData::I32(v) => v.map(Into::into).unwrap_or(serde_json::Value::Null),
        ColumnData::I64(v) => v.map(Into::into).unwrap_or(serde_json::Value::Null),
        ColumnData::F32(v) => v.map(|n| serde_json::Value::from(n as f64)).unwrap_or(serde_json::Value::Null),
        ColumnData::F64(v) => v.map(Into::into).unwrap_or(serde_json::Value::Null),
        ColumnData::Bit(v) => v.map(Into::into).unwrap_or(serde_json::Value::Null),
        // Rendered as an exact decimal string via `format_numeric` (not
        // `f64`, for the same precision reason binding avoids `f64` -- see
        // module doc; not `Numeric`'s own `Display`, see `format_numeric`'s
        // doc comment).
        ColumnData::Numeric(v) => v
            .map(|n| serde_json::Value::String(format_numeric(n)))
            .unwrap_or(serde_json::Value::Null),
        ColumnData::String(v) => v
            .as_ref()
            .map(|s| serde_json::Value::String(s.to_string()))
            .unwrap_or(serde_json::Value::Null),
        ColumnData::Guid(v) => v
            .map(|g| serde_json::Value::String(g.to_string()))
            .unwrap_or(serde_json::Value::Null),
        ColumnData::Binary(v) => v
            .as_ref()
            .map(|b| {
                serde_json::Value::String(base64::Engine::encode(
                    &base64::engine::general_purpose::STANDARD,
                    b.as_ref(),
                ))
            })
            .unwrap_or(serde_json::Value::Null),
        ColumnData::Xml(v) => v
            .as_ref()
            .map(|x| serde_json::Value::String(x.to_string()))
            .unwrap_or(serde_json::Value::Null),
        ColumnData::DateTime(_) | ColumnData::SmallDateTime(_) | ColumnData::DateTime2(_) => {
            temporal_to_json(data)
        }
        ColumnData::Time(_) => temporal_to_json(data),
        ColumnData::Date(_) => temporal_to_json(data),
        ColumnData::DateTimeOffset(_) => temporal_to_json(data),
    }
}

/// `ColumnData`'s temporal variants (`DateTime`/`SmallDateTime`/`Time`/
/// `Date`/`DateTime2`/`DateTimeOffset`) don't expose a ready-made chrono
/// conversion on the enum itself — only `FromSql` (via `Row::get::<T,_>`)
/// does, keyed by the target Rust/chrono type, and `Row` isn't
/// constructible outside the crate. This duplicates tiberius's own
/// `tds::time::chrono` conversion arithmetic (days-since-epoch + 100ns
/// increments) for the one-way read path instead.
fn temporal_to_json(data: &ColumnData<'_>) -> serde_json::Value {
    fn days_to_date(days: i64, start_year: i32) -> chrono::NaiveDate {
        chrono::NaiveDate::from_ymd_opt(start_year, 1, 1).unwrap() + chrono::Duration::days(days)
    }
    fn time_of_day(increments: u64, scale: u8) -> chrono::NaiveTime {
        let ns = increments as i64 * 10i64.pow(9 - scale as u32);
        chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap() + chrono::Duration::nanoseconds(ns)
    }

    let text = match data {
        ColumnData::Date(Some(date)) => days_to_date(date.days() as i64, 1).format("%Y-%m-%d").to_string(),
        ColumnData::Time(Some(time)) => time_of_day(time.increments(), time.scale())
            .format("%H:%M:%S%.f")
            .to_string(),
        ColumnData::SmallDateTime(Some(dt)) => {
            let date = days_to_date(dt.days() as i64, 1900);
            let time = chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap()
                + chrono::Duration::minutes(dt.seconds_fragments() as i64);
            chrono::NaiveDateTime::new(date, time)
                .format("%Y-%m-%dT%H:%M:%S")
                .to_string()
        }
        ColumnData::DateTime(Some(dt)) => {
            let date = days_to_date(dt.days() as i64, 1900);
            let ns = dt.seconds_fragments() as i64 * 1_000_000_000 / 300;
            let time = chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap() + chrono::Duration::nanoseconds(ns);
            chrono::NaiveDateTime::new(date, time)
                .format("%Y-%m-%dT%H:%M:%S%.f")
                .to_string()
        }
        ColumnData::DateTime2(Some(dt2)) => {
            let date = days_to_date(dt2.date().days() as i64, 1);
            let time = time_of_day(dt2.time().increments(), dt2.time().scale());
            chrono::NaiveDateTime::new(date, time)
                .format("%Y-%m-%dT%H:%M:%S%.f")
                .to_string()
        }
        ColumnData::DateTimeOffset(Some(dto)) => {
            let dt2 = dto.datetime2();
            let date = days_to_date(dt2.date().days() as i64, 1);
            let time = time_of_day(dt2.time().increments(), dt2.time().scale());
            let naive = chrono::NaiveDateTime::new(date, time);
            let offset = chrono::FixedOffset::east_opt(dto.offset() as i32 * 60).unwrap();
            naive
                .and_utc()
                .with_timezone(&offset)
                .to_rfc3339_opts(chrono::SecondsFormat::AutoSi, false)
        }
        _ => return serde_json::Value::Null,
    };
    serde_json::Value::String(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_type_strips_length_and_lowercases() {
        assert_eq!(base_type("NVARCHAR(256)"), "nvarchar");
        assert_eq!(base_type("decimal(18,2)"), "decimal");
        assert_eq!(base_type("int"), "int");
    }

    #[test]
    fn precision_scale_parses_both_numbers() {
        assert_eq!(precision_scale("decimal(18,2)"), (18, 2));
        assert_eq!(precision_scale("numeric(38,10)"), (38, 10));
        assert_eq!(precision_scale("decimal"), (18, 0));
    }

    #[test]
    fn parse_numeric_round_trips_a_negative_decimal() {
        let n = parse_numeric("-123.45", 2).unwrap();
        assert_eq!(format_numeric(n), "-123.45");
    }

    #[test]
    fn parse_numeric_pads_missing_fractional_digits() {
        let n = parse_numeric("5", 2).unwrap();
        assert_eq!(format_numeric(n), "5.00");
    }

    #[test]
    fn parse_numeric_rejects_too_many_fractional_digits() {
        assert!(parse_numeric("1.2345", 2).is_err());
    }

    #[test]
    fn format_numeric_handles_zero_and_positive_values() {
        assert_eq!(format_numeric(parse_numeric("0", 2).unwrap()), "0.00");
        assert_eq!(format_numeric(parse_numeric("7.5", 2).unwrap()), "7.50");
        assert_eq!(format_numeric(parse_numeric("-0.01", 2).unwrap()), "-0.01");
    }

    #[test]
    fn json_to_param_handles_int_and_string() {
        json_to_param(&serde_json::json!(42), "int").unwrap();
        json_to_param(&serde_json::json!("hello"), "nvarchar(50)").unwrap();
        json_to_param(&serde_json::Value::Null, "int").unwrap();
    }
}
