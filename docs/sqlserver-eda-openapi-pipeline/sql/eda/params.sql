/*
 * params.sql
 *
 * Parameter metadata (name, type, direction, default) for every object
 * objects.sql would match (is_ms_shipped = 1, the same operation type set),
 * from sys.all_parameters (covers both user and system objects;
 * sys.system_parameters is a subset view over the same rows for system
 * objects and is not needed separately).
 *
 * parameter_id = 0 (the function-return-value pseudo-row functions carry)
 * is excluded -- it isn't a real input/output parameter, and dropping it
 * here keeps this script's output symmetric with resultset.sql's/
 * resultset_fmtonly.sql's dynamic-call construction, which also skips it.
 *
 * Run the same way as objects.sql, once per database per version:
 *   sqlcmd -S localhost,<port> -U sa -P "$MSSQL_SA_PASSWORD" -C \
 *     -v db=<master|msdb|model> -i sql/eda/params.sql -o data/<version>/<db>.params.json
 */

-- See objects.sql for why this is an explicit USE off a required sqlcmd
-- scripting variable rather than relying on sqlcmd's -d connection flag.
USE $(db);

SET NOCOUNT ON;

SELECT
    DB_NAME()                              AS database_name,
    SCHEMA_NAME(o.schema_id)                AS schema_name,
    o.name                                  AS object_name,
    o.type                                  AS object_type_code,
    o.type_desc                             AS object_type_desc,
    p.name                                  AS parameter_name,
    p.parameter_id                          AS ordinal,
    t.name                                  AS data_type,
    p.max_length                            AS max_length,
    p.precision                             AS precision,
    p.scale                                 AS scale,
    p.is_output                             AS is_output,
    p.is_cursor_ref                         AS is_cursor_ref,
    p.has_default_value                     AS has_default_value,
    CASE WHEN p.has_default_value = 1
         THEN CONVERT(nvarchar(4000), p.default_value)
         ELSE NULL
    END                                      AS default_value
FROM sys.all_objects AS o
JOIN sys.all_parameters AS p
    ON p.object_id = o.object_id
JOIN sys.types AS t
    ON t.user_type_id = p.user_type_id
WHERE o.is_ms_shipped = 1
  AND o.type IN ('P', 'PC', 'X', 'FN', 'IF', 'TF', 'FS', 'FT', 'V')
  AND p.parameter_id > 0
ORDER BY schema_name, object_name, ordinal
FOR JSON PATH;
