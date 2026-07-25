#!/usr/bin/env python3
"""Turn EDA extraction output (data/<version>/<db>.{objects,params,resultset,
resultset_fmtonly}.{json,txt}) into one synthetic OpenAPI 3.1 YAML file per
version (openapi/<version>/combined.yaml), one POST operation per documented
stored procedure/function/view, deduplicated across master/msdb/model.

Usage:
    tools/generate_openapi.py <version>
    tools/generate_openapi.py 2022
"""
from __future__ import annotations

import copy
import json
import re
import sys
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parent.parent

# Databases swept by sql/eda/*.sql, in merge-tiebreak priority order (first
# db with usable data for a given field wins when candidates are otherwise
# equally good -- see choose_best_resultset()/first_nonempty()). `model` is
# SQL Server's own template database for new user databases -- the closest
# thing to a generic "non-system user database" that's guaranteed to exist
# on every instance, replacing the old ad hoc `sandbox` database.
DATABASES = ("master", "msdb", "model")

# How many operations survive the rank cut per version -- see rank_key()/
# main(). The broad is_ms_shipped sweep this pipeline now runs (see
# objects.sql) turns up far more than 500 candidates on a real instance;
# this keeps the generated spec (and the MCP server built from it) to a
# reviewable, genuinely useful subset instead of dumping every internal
# system object SQL Server happens to ship.
TOP_N_PER_VERSION = 500

# The synthetic per-operation parameter documenting which database to run an
# operation against. It is NOT a real parameter of the underlying SQL Server
# object -- it never appears in sys.all_parameters -- so it's added here,
# after introspection, to every operation's request body (see
# build_request_schema()). It has to be a real documented request-body
# property, not an OpenAPI vendor extension (`x-sql-...`), because mcpify's
# generated store does not carry vendor extensions through into the schemas
# it actually validates calls against (see services/api_client.rs's own
# doc comment on this same limitation) -- only real `properties` survive.
EXECUTION_DATABASE_PARAM = "execution_database"


# SQL Server's FOR JSON splits output longer than 2,033 characters across
# multiple actual result-set rows (each up to 2,033 chars), not just one long
# string -- this is documented FOR JSON behavior, independent of sqlcmd. Each
# row lands on its own physical line in the output file, so reassembly means
# concatenating every line's content with the newlines *removed* (a bare
# newline is never valid inside FOR JSON's escaped string output, so any
# newline in the file is an artifact of sqlcmd's one-row-per-line writing,
# not real content). Also drop anything before the first `[`/`{`, in case a
# column header line/dashes separator precedes the data (e.g. if sqlcmd was
# invoked without `-h -1`).
def load_json_dump(path: Path) -> list[dict]:
    if not path.exists():
        return []
    raw = path.read_text(encoding="utf-8-sig")
    text = raw.replace("\r\n", "\n").replace("\n", "").strip()
    if not text:
        return []
    start = min((i for i in (text.find("["), text.find("{")) if i != -1), default=-1)
    if start == -1:
        raise ValueError(f"{path}: no JSON array/object found in sqlcmd output")
    return json.loads(text[start:])


# resultset_fmtonly.sql's output is sqlcmd's own plain-text result-set
# rendering (see that script's header comment for why: FMTONLY's column
# metadata is a wire-protocol response to the *client*, not something a
# T-SQL session can SELECT back out of itself), not JSON. Each object's
# block is introduced by a `===OBJECT:schema.name:TYPE===` marker (from a
# `PRINT` in the SQL script) and, when the FMTONLY-mode statement produced a
# result set, followed by sqlcmd's normal header line + a dashes separator
# line (`------ ------ ...`) -- the header line splits cleanly on whitespace
# since SQL Server column identifiers can never contain spaces. A block
# containing an `ERROR: ...` line instead means the TRY/CATCH in the SQL
# script caught a failure for that object; a block with neither means
# FMTONLY produced no result set at all for it.
_OBJECT_MARKER_RE = re.compile(r"^===OBJECT:([^.]+)\.(.+):([A-Za-z]{1,2})===$")
_SEPARATOR_LINE_RE = re.compile(r"^-+( +-+)*$")


def load_fmtonly_dump(path: Path) -> dict[tuple[str, str], dict]:
    if not path.exists():
        return {}
    text = path.read_text(encoding="utf-8-sig").replace("\r\n", "\n")

    result: dict[tuple[str, str], dict] = {}
    current_key: tuple[str, str] | None = None
    current_lines: list[str] = []

    def flush() -> None:
        if current_key is None:
            return
        columns: list[str] | None = None
        error: str | None = None
        for i, line in enumerate(current_lines):
            stripped = line.strip()
            if stripped.startswith("ERROR:"):
                error = stripped[len("ERROR:"):].strip()
                break
            if i > 0 and stripped and _SEPARATOR_LINE_RE.match(stripped):
                header = current_lines[i - 1].strip()
                if header:
                    columns = header.split()
                break
        result[current_key] = {"columns": columns, "error": error}

    for raw_line in text.split("\n"):
        line = raw_line.rstrip()
        m = _OBJECT_MARKER_RE.match(line.strip())
        if m:
            flush()
            current_key = (m.group(1), m.group(2))
            current_lines = []
        elif current_key is not None:
            current_lines.append(line)
    flush()
    return result


# Coarse SQL Server type -> OpenAPI (type, format) mapping. Anything not
# listed falls back to {"type": "string"} rather than guessing further.
SQL_TO_OPENAPI = {
    "int": ("integer", "int32"),
    "bigint": ("integer", "int64"),
    "smallint": ("integer", "int32"),
    "tinyint": ("integer", "int32"),
    "bit": ("boolean", None),
    "decimal": ("number", "double"),
    "numeric": ("number", "double"),
    "float": ("number", "double"),
    "real": ("number", "float"),
    "money": ("number", "double"),
    "smallmoney": ("number", "double"),
    "datetime": ("string", "date-time"),
    "datetime2": ("string", "date-time"),
    "smalldatetime": ("string", "date-time"),
    "date": ("string", "date"),
    "time": ("string", None),
    "datetimeoffset": ("string", "date-time"),
    "uniqueidentifier": ("string", "uuid"),
    "varchar": ("string", None),
    "nvarchar": ("string", None),
    "char": ("string", None),
    "nchar": ("string", None),
    "text": ("string", None),
    "ntext": ("string", None),
    "xml": ("string", None),
    "sql_variant": ("string", None),
    "varbinary": ("string", "byte"),
    "binary": ("string", "byte"),
    "image": ("string", "byte"),
    "cursor": ("string", None),
    "table type": ("array", None),
}

# Hand-curated one-line summaries for well-known objects. This is prose
# only -- unlike the old CURATED_PARAMETERS dict this file used to carry,
# nothing here stands in for introspected parameter/column metadata; system
# object metadata just has no reliable description field to pull a summary
# from automatically (see README limitations), so a small hand-written set
# covers the objects most worth explaining well.
SUMMARIES = {
    "sp_who": "List active SQL Server user connections/processes and what they're blocking.",
    "sp_who2": "Extended, more readable version of sp_who.",
    "sp_help": "Report metadata about a database object (or all objects if no name given).",
    "sp_helpdb": "Report information about one or all databases.",
    "sp_helptext": "Return the definition text of a view, procedure, trigger, or function.",
    "sp_helpindex": "List the indexes defined on a table or view.",
    "sp_helpconstraint": "List the constraints defined on a table.",
    "sp_columns": "Return column metadata for a table or view, ODBC-catalog style.",
    "sp_tables": "Return the list of tables/views available in the current environment.",
    "sp_stored_procedures": "Return the list of stored procedures in the current environment.",
    "sp_databases": "List databases available on the server (or via linked server).",
    "sp_server_info": "Return server attribute/value pairs describing the instance.",
    "sp_configure": "Display or change server-wide configuration options.",
    "sp_rename": "Rename a table, column, index, or other user object.",
    "sp_executesql": "Execute a Transact-SQL statement or batch with parameterized substitution.",
    "sp_execute": "Execute a previously prepared statement handle.",
    "sp_prepare": "Prepare a Transact-SQL statement and return a handle for repeated execution.",
    "sp_unprepare": "Release resources for a statement handle created by sp_prepare.",
    "sp_addlinkedserver": "Register a linked server for distributed queries.",
    "sp_droplinkedserver": "Remove a linked server registration.",
    "sp_linkedservers": "List all linked servers registered on the instance.",
    "sp_addrole": "Create a new database role.",
    "sp_addrolemember": "Add a database user to a database role.",
    "sp_addlogin": "Create a new SQL Server login (legacy; sp_addlogin is deprecated in favor of CREATE LOGIN).",
    "sp_grantdbaccess": "Grant a login access to the current database (legacy; deprecated in favor of CREATE USER).",
    "sp_depends": "List the objects that depend on, or are depended on by, a given object (deprecated in favor of sys.dm_sql_referencing_entities).",
    "sp_lock": "Report information about currently held locks (legacy; deprecated in favor of sys.dm_tran_locks).",
    "sp_monitor": "Display SQL Server usage statistics since the last call.",
    "sp_spaceused": "Report the disk space used by a table or the whole database.",
    "sp_estimate_data_compression_savings": "Estimate the space saved by applying row/page compression to a table or index.",
    "sp_set_session_context": "Set a key-value pair in the current session context, readable via SESSION_CONTEXT().",
    "sp_describe_first_result_set": "Describe the shape of the first result set a Transact-SQL statement would return.",
    "sp_describe_undeclared_parameters": "Describe the parameters expected by a Transact-SQL statement.",
    "sp_msforeachtable": "Run a command once for each table in the database (undocumented but widely used).",
    "sp_msforeachdb": "Run a command once for each database on the server (undocumented but widely used).",
    "sp_add_job": "Create a new SQL Server Agent job.",
    "sp_add_jobstep": "Add a step to a SQL Server Agent job.",
    "sp_add_jobschedule": "Attach a schedule to a SQL Server Agent job.",
    "sp_add_schedule": "Create a reusable SQL Server Agent schedule.",
    "sp_start_job": "Start a SQL Server Agent job immediately.",
    "sp_stop_job": "Stop a currently running SQL Server Agent job.",
    "sp_delete_job": "Delete a SQL Server Agent job.",
    "sp_help_job": "Report information about one or all SQL Server Agent jobs.",
    "sp_help_jobstep": "Report information about the steps of a SQL Server Agent job.",
    "sp_help_jobschedule": "Report information about the schedules attached to a SQL Server Agent job.",
    "sp_help_schedule": "Report information about SQL Server Agent schedules.",
    "sp_helphistory": "Report the run history of a SQL Server Agent job.",
}


def object_summary(name: str, type_desc: str) -> str:
    if name in SUMMARIES:
        return SUMMARIES[name]
    # `name` is a real SQL Server identifier (e.g. "all_parameters") and
    # must stay underscored everywhere else (operationId, dict keys) --
    # but this summary text is exactly what gets embedded for semantic
    # search (see services/embedding_service.rs), and fastembed's models
    # are trained on natural English prose where words are space-separated.
    # "all_parameters" as one underscore-joined token embeds worse against
    # a query like "show me all parameters" than "all parameters" as two
    # real words does, so only this human-readable rendering swaps
    # underscores for spaces.
    readable = name.replace("_", " ")
    if type_desc.startswith("SQL_STORED_PROCEDURE") or type_desc == "CLR_STORED_PROCEDURE":
        return f"System stored procedure {readable} (see Microsoft Learn for details)."
    if type_desc == "EXTENDED_STORED_PROCEDURE":
        return f"Extended stored procedure {readable} (see Microsoft Learn for details)."
    if "TABLE_VALUED_FUNCTION" in type_desc or type_desc.endswith("_FUNCTION"):
        return f"System function {readable} (see Microsoft Learn for details)."
    if type_desc == "VIEW":
        return f"System/catalog view {readable} (see Microsoft Learn for details)."
    return f"System object {readable}."


# SQL Server types whose max_length is reported in bytes rather than
# characters (2 bytes/char for the Unicode "n" variants).
_UNICODE_STRING_TYPES = {"nvarchar", "nchar", "ntext"}
_BYTE_LENGTH_STRING_TYPES = {"varchar", "char", "varbinary", "binary", "text", "image"}
_PRECISION_SCALE_TYPES = {"decimal", "numeric"}
_SCALE_ONLY_TYPES = {"time", "datetime2", "datetimeoffset"}


def format_sql_type(
    data_type: str,
    max_length: int | None = None,
    precision: int | None = None,
    scale: int | None = None,
) -> str:
    """Render a SQL Server type name plus its length/precision/scale exactly
    as you'd write it in a CREATE TABLE/CREATE PROC statement, e.g.
    "nvarchar(256)", "decimal(18,2)", "datetime2(7)", "int". Used as the
    x-sql-type annotation on every generated schema property so the JSON
    property can be mapped back to its exact SQL Server column/parameter type.
    """
    dt = data_type.lower()
    if dt in _PRECISION_SCALE_TYPES and precision is not None:
        return f"{data_type}({precision},{scale or 0})"
    if dt in _SCALE_ONLY_TYPES and scale:
        return f"{data_type}({scale})"
    if dt in _UNICODE_STRING_TYPES:
        if max_length is None:
            return data_type
        return f"{data_type}(max)" if max_length == -1 else f"{data_type}({max_length // 2})"
    if dt in _BYTE_LENGTH_STRING_TYPES:
        if max_length is None:
            return data_type
        return f"{data_type}(max)" if max_length == -1 else f"{data_type}({max_length})"
    return data_type


def sql_type_to_schema(
    data_type: str,
    max_length: int | None = None,
    precision: int | None = None,
    scale: int | None = None,
    sql_type_display: str | None = None,
) -> dict:
    """Build an OpenAPI schema fragment for a SQL Server type. Always carries
    an `x-sql-type` field with the exact SQL type text (e.g. "nvarchar(256)")
    alongside the best-effort OpenAPI type/format, since the OpenAPI type
    system is too coarse to round-trip SQL Server types on its own (e.g.
    every string-like type maps to `type: string`, every exact/approximate
    numeric type maps to `type: number`).
    """
    oapi_type, fmt = SQL_TO_OPENAPI.get(data_type.lower(), ("string", None))
    schema: dict = {"type": oapi_type}
    if fmt:
        schema["format"] = fmt
    if oapi_type == "array":
        schema["items"] = {"type": "object"}
    schema["x-sql-type"] = sql_type_display or format_sql_type(data_type, max_length, precision, scale)
    return schema


def build_request_schema(params: list[dict], found_in_databases: list[str]) -> dict:
    """Build the request-body schema for one operation: its real (introspected)
    input parameters, plus the synthetic EXECUTION_DATABASE_PARAM every
    operation gets regardless of whether it has any real parameters at all --
    see that constant's own doc comment for why this can't be a vendor
    extension instead. Never returns None: every operation now has a request
    body, even a fully parameterless one, because the execution-context
    parameter is always present.
    """
    input_params = [p for p in params if not p.get("is_output")]
    properties: dict = {}
    required: list[str] = []
    for p in sorted(input_params, key=lambda p: p["ordinal"]):
        name = p["parameter_name"].lstrip("@") if p["parameter_name"] else f"param{p['ordinal']}"
        schema = sql_type_to_schema(p["data_type"], p.get("max_length"), p.get("precision"), p.get("scale"))
        if p.get("default_value") is not None:
            schema["default"] = p["default_value"]
        # Explicit, since dict/JSON-object property order is not a
        # contract a downstream consumer can safely rely on to recover
        # declared parameter order (e.g. positional table-valued-function
        # calls, where argument order is significant and there is no named-
        # parameter call syntax) once this schema has been serialized,
        # merged, and re-deserialized through tools that don't all
        # guarantee insertion-order-preserving JSON maps.
        schema["x-sql-ordinal"] = p["ordinal"]
        properties[name] = schema
        if not p.get("has_default_value"):
            required.append(name)

    properties[EXECUTION_DATABASE_PARAM] = {
        "type": "string",
        "description": (
            "Optional execution context: the database to run this operation against "
            "(SQL Server two/three-part-name qualification). This is NOT a real "
            "parameter of the underlying SQL Server object -- it never appears in "
            "sys.all_parameters for it -- it's added by tools/generate_openapi.py to "
            "every operation. This object was found, identically, in: "
            f"{', '.join(found_in_databases)}. If omitted, the server's configured "
            "default database is used if one was set up (optional, see setup); "
            "otherwise the connection's own current database context applies "
            "(equivalent to DB_NAME()/DB_ID())."
        ),
        "x-sql-synthetic": True,
    }

    body: dict = {"type": "object", "properties": properties}
    if required:
        body["required"] = required
    return body


def build_output_param_schema(params: list[dict]) -> dict | None:
    output_params = [p for p in params if p.get("is_output")]
    if not output_params:
        return None
    properties = {}
    for p in sorted(output_params, key=lambda p: p["ordinal"]):
        name = p["parameter_name"].lstrip("@") if p["parameter_name"] else f"param{p['ordinal']}"
        properties[name] = sql_type_to_schema(p["data_type"], p.get("max_length"), p.get("precision"), p.get("scale"))
    return {"type": "object", "properties": properties}


def build_response_schema(resultset: dict) -> dict:
    """Build the 200 response schema from choose_best_resultset()'s pick.
    `resultset["tier"]` is one of "described" (fully typed, from
    sys.dm_exec_describe_first_result_set), "fmtonly" (column names only,
    from resultset_fmtonly.sql's SET FMTONLY ON fallback), "no_result_set",
    or "unknown".
    """
    tier = resultset["tier"]
    if tier == "described":
        rows = resultset["rows"]
        properties = {}
        for r in sorted(rows, key=lambda r: (r.get("column_ordinal") or 0)):
            if not r.get("column_name"):
                continue
            system_type_name = r.get("system_type_name") or ""
            base_type = system_type_name.split("(")[0]
            # sys.dm_exec_describe_first_result_set's system_type_name already
            # includes length/precision/scale (e.g. "nchar(30)", "decimal(18,2)"),
            # so use it verbatim as x-sql-type instead of re-deriving it.
            schema = sql_type_to_schema(base_type, sql_type_display=system_type_name or base_type)
            properties[r["column_name"]] = schema
        return {"type": "array", "items": {"type": "object", "properties": properties}}
    if tier == "fmtonly":
        properties = {
            name: {"type": "string", "x-sql-type": "unknown (name-only, see x-sql-columns-source)"}
            for name in resultset["columns"]
        }
        return {
            "type": "array",
            "items": {"type": "object", "properties": properties},
            "x-sql-columns-source": "fmtonly",
            "description": (
                "Column names recovered via SET FMTONLY ON (sys.dm_exec_describe_first_result_set "
                "could not describe this object's result set). Names only -- FMTONLY's column "
                "metadata is a wire-protocol response to the connecting client, not something a "
                "T-SQL session can query back typed information from; see resultset_fmtonly.sql."
            ),
        }
    if tier == "no_result_set":
        return {"type": "object", "description": "This object does not return a result set."}
    # tier == "unknown"
    desc = "Result set shape could not be determined by introspection"
    if resultset.get("error"):
        desc += f": {resultset['error']}"
    return {"type": "object", "description": desc}


# Authentication modes the SQL Server *engine* itself accepts for a
# connection (not an HTTP auth scheme in reality -- these are TDS-protocol
# login types). Mapped to the closest-fitting OpenAPI securityScheme shape so
# tooling that consumes the spec at least knows which credential types are
# valid for a given version. `versions` lists the engine versions (as passed
# to this script) that support the mode.
SECURITY_SCHEMES = {
    "sqlAuth": {
        "versions": {"2017", "2019", "2022", "2025"},
        "scheme": {
            "type": "http",
            "scheme": "basic",
            "description": (
                "SQL Server Authentication: a SQL login (username + password) validated by the "
                "engine itself, e.g. `sqlcmd -U sa -P <password>`. Available in every version, "
                "but only usable when the instance is configured for Mixed Mode (SQL Server and "
                "Windows Authentication); Windows-Authentication-only instances reject it."
            ),
        },
    },
    "windowsAuth": {
        "versions": {"2017", "2019", "2022", "2025"},
        "scheme": {
            "type": "http",
            "scheme": "negotiate",
            "description": (
                "Windows Authentication (Integrated Security): the client's Windows/Kerberos or "
                "NTLM identity is passed through instead of a SQL login, e.g. `sqlcmd -E`. "
                "Available in every version; on Linux containers this requires the container to be "
                "configured for Kerberos (keytab + krb5.conf) since there is no NTLM/domain-join "
                "support otherwise."
            ),
        },
    },
    "azureADAuth": {
        # Azure AD (Microsoft Entra ID) authentication for on-premises/Linux
        # SQL Server (as opposed to Azure SQL Database, which has always
        # supported it) was introduced as a new engine feature in SQL Server
        # 2022 -- 2017/2019 cannot authenticate this way at all.
        "versions": {"2022", "2025"},
        "scheme": {
            "type": "oauth2",
            "description": (
                "Azure Active Directory (Microsoft Entra ID) authentication. Added as an engine "
                "feature for on-premises/Linux SQL Server in SQL Server 2022 (previously "
                "Azure-AD-only offering was limited to Azure SQL Database/Managed Instance); not "
                "available on SQL Server 2017 or 2019."
            ),
            "flows": {
                "authorizationCode": {
                    "authorizationUrl": "https://login.microsoftonline.com/common/oauth2/v2.0/authorize",
                    "tokenUrl": "https://login.microsoftonline.com/common/oauth2/v2.0/token",
                    "scopes": {"https://database.windows.net/.default": "Access SQL Server as the signed-in Azure AD identity"},
                }
            },
        },
    },
}


# SQL Server surfaces execution failures (constraint violations, RAISERROR,
# THROW, permission checks, conversion errors, resource/fatal errors, ...) as
# an error on the TDS connection -- never as an HTTP status, so any mapping to
# 4xx/5xx is again a synthetic convention, same as the POST-per-object
# mapping itself. The convention here follows the engine's own severity
# levels (sys.messages.severity / THROW's severity argument / RAISERROR's
# severity argument / SqlException.Class), which is the actual thing that
# already distinguishes "the caller did something wrong" from "the server is
# broken":
#   - Severity 11-16: statement/user errors the caller can fix by changing
#     input (constraint violations, conversion errors, explicit RAISERROR or
#     THROW at default severity 16, "invalid object name", etc.) -> 400.
#   - Msg 229/230/262/300-series ("... permission was denied") are severity
#     14 but specifically about authorization rather than malformed input, so
#     they get their own 403 rather than folding into 400.
#   - Severity 17-25: resource errors, hardware/software faults, or fatal
#     errors that tear down the connection -- not something the caller can
#     fix by resubmitting -> 500.
# THROW re-raising a caught error (no arguments) preserves the original
# error's number/severity/state, so it lands in whichever bucket the
# original error was already in.
SQL_ERROR_SCHEMA_NAME = "SqlServerError"
SQL_ERROR_SCHEMA = {
    "type": "object",
    "description": (
        "Shape of a SQL Server engine error as surfaced on the TDS connection (the fields "
        "available from THROW / RAISERROR / ERROR_NUMBER() / ERROR_SEVERITY() / ERROR_STATE() / "
        "ERROR_PROCEDURE() / ERROR_LINE() / ERROR_MESSAGE(), or a client driver's exception object)."
    ),
    "properties": {
        "number": {"type": "integer", "x-sql-type": "int", "description": "ERROR_NUMBER() -- e.g. 50000 for a user RAISERROR/THROW with no explicit error number."},
        "severity": {"type": "integer", "x-sql-type": "int", "description": "ERROR_SEVERITY() -- see the severity-to-status mapping on each operation's error responses."},
        "state": {"type": "integer", "x-sql-type": "int", "description": "ERROR_STATE() -- caller-defined, used to distinguish multiple raise points of the same error number."},
        "procedure": {"type": ["string", "null"], "x-sql-type": "nvarchar(128)", "description": "ERROR_PROCEDURE() -- null if the error was raised in a batch, not inside a procedure."},
        "line": {"type": ["integer", "null"], "x-sql-type": "int", "description": "ERROR_LINE()"},
        "message": {"type": "string", "x-sql-type": "nvarchar(2048)", "description": "ERROR_MESSAGE()"},
    },
    "required": ["number", "severity", "state", "message"],
}


def build_error_responses() -> dict:
    """Shared 400/403/500 response objects (all operations reference the same
    SqlServerError schema), keyed as OpenAPI status codes.
    """
    # Each status gets its own literal $ref dict rather than one shared `ref`
    # reused three times -- reusing one object here would make deepcopy's
    # aliasing-preservation reintroduce the exact &anchor/*alias problem this
    # function's per-operation deep copy (in main()) is trying to avoid.
    def ref() -> dict:
        return {"$ref": f"#/components/schemas/{SQL_ERROR_SCHEMA_NAME}"}

    return {
        "400": {
            "description": (
                "Statement/user error the caller can fix by changing input -- SQL Server severity "
                "11-16 (constraint violation, conversion error, invalid object/column name, "
                "explicit RAISERROR or THROW at the default severity 16, etc.)."
            ),
            "content": {"application/json": {"schema": ref()}},
        },
        "403": {
            "description": (
                "Permission denied -- SQL Server severity 14 errors specifically about "
                "authorization (e.g. Msg 229/230, \"The EXECUTE/SELECT permission was denied on "
                "the object ...\"), as opposed to other severity-14 statement errors (400)."
            ),
            "content": {"application/json": {"schema": ref()}},
        },
        "500": {
            "description": (
                "Resource, hardware/software, or fatal engine error -- SQL Server severity 17-25 "
                "-- not something the caller can fix by resubmitting the same request."
            ),
            "content": {"application/json": {"schema": ref()}},
        },
    }


def build_security(version: str) -> tuple[dict, list]:
    """Return (securitySchemes, security) for the given engine version. Each
    entry in `security` is its own single-scheme requirement, which OpenAPI
    treats as "satisfies ANY one of these" -- i.e. "this version accepts SQL
    auth OR Windows auth OR (if applicable) Azure AD auth", not all three at
    once.
    """
    schemes = {}
    security = []
    for name, entry in SECURITY_SCHEMES.items():
        if version in entry["versions"]:
            schemes[name] = entry["scheme"]
            security.append({name: []})
    return schemes, security


# --- Cross-version presence ------------------------------------------------

def discover_versions(data_root: Path) -> list[str]:
    if not data_root.exists():
        return []
    return sorted(p.name for p in data_root.iterdir() if p.is_dir())


def compute_presence(data_root: Path) -> tuple[dict[tuple[str, str, str], set[str]], int]:
    """For every version with extracted data sitting in data/<version>/,
    which (schema, name, type) identities exist in ANY of master/msdb/model.
    Used by rank_key() to prefer operations that exist across as many
    versions as possible, so the top-{TOP_N_PER_VERSION} cut lands on
    roughly the same operation set on every version -- a prompt/workflow
    written against one version's generated tools keeps working when
    switched to another, instead of hitting an operation that only survived
    the rank cut on some versions. This only helps when every version's
    data/<version>/ has already been extracted before generation runs (see
    scripts/regenerate_mcp_server.sh's ordering); with only one version's
    data present, every object's presence count is trivially 1 and ranking
    degrades to metadata-completeness-first, same as before this existed.
    """
    versions = discover_versions(data_root)
    presence: dict[tuple[str, str, str], set[str]] = {}
    for v in versions:
        seen_this_version: set[tuple[str, str, str]] = set()
        for db in DATABASES:
            for obj in load_json_dump(data_root / v / f"{db}.objects.json"):
                seen_this_version.add((obj["schema_name"], obj["object_name"], obj["object_type_desc"]))
        for key in seen_this_version:
            presence.setdefault(key, set()).add(v)
    return presence, len(versions)


# --- Cross-database merge/dedup -------------------------------------------

# Object-type tier for ranking (build_operations()/rank_key()): stored
# procedures first (the most directly "operation"-shaped, and most system
# procs are what this project's users actually want to call), then
# functions, then views (catalog views/INFORMATION_SCHEMA/DMVs -- queryable,
# but read-only metadata surfaces rather than callable operations).
_TYPE_TIER = {
    "SQL_STORED_PROCEDURE": 0,
    "CLR_STORED_PROCEDURE": 0,
    "EXTENDED_STORED_PROCEDURE": 0,
    "SQL_SCALAR_FUNCTION": 1,
    "SQL_INLINE_TABLE_VALUED_FUNCTION": 1,
    "SQL_TABLE_VALUED_FUNCTION": 1,
    "CLR_SCALAR_FUNCTION": 1,
    "CLR_TABLE_VALUED_FUNCTION": 1,
    "VIEW": 2,
}


def first_nonempty(candidates: list[list[dict]]) -> list[dict]:
    for c in candidates:
        if c:
            return c
    return []


def candidate_resultset_tier(rows: list[dict] | None, fmtonly: dict | None) -> tuple[int, dict]:
    """Rank one database's resultset evidence for a single object, highest
    tier first: 4 = fully typed (sys.dm_exec_describe_first_result_set
    succeeded AND at least one described column actually has a name --
    resultset.sql inserts a 'described' row whenever the describe call
    returned *any* rows, but for some extended stored procedures it returns
    row(s) with a NULL column_name/ordinal instead of failing outright
    (observed live: `sp_executesql`, an EXTENDED_STORED_PROCEDURE, "described"
    two anonymous columns) -- FOR JSON PATH drops NULL-valued keys entirely,
    so those rows arrive here with no "column_name" key at all, and without
    this check they'd be misread as real columns), 3 = column names only
    (the resultset_fmtonly.sql fallback succeeded where the primary pass
    didn't), 2 = confirmed no result set, 1 = introspection failed (status
    'unknown'), 0 = no evidence at all for this database.
    """
    status = rows[0].get("result_set_status") if rows else None
    if status == "described" and any(r.get("column_name") for r in rows):
        return 4, {"tier": "described", "rows": rows}
    if fmtonly and fmtonly.get("columns"):
        return 3, {"tier": "fmtonly", "columns": fmtonly["columns"]}
    if status == "no_result_set" or status == "described":
        return 2, {"tier": "no_result_set"}
    if status == "unknown":
        return 1, {"tier": "unknown", "error": rows[0].get("error_message") if rows else None}
    return 0, {"tier": "unknown", "error": None}


def choose_best_resultset(candidates: dict[str, dict]) -> dict:
    best_rank = -1
    best: dict = {"tier": "unknown", "error": None}
    for db in DATABASES:
        c = candidates.get(db)
        if not c:
            continue
        rank, info = candidate_resultset_tier(c.get("resultset_rows"), c.get("fmtonly"))
        if rank > best_rank:
            best_rank, best = rank, info
    return best


def build_operations(version: str, data_dir: Path) -> tuple[list[dict], int]:
    """Load master/msdb/model EDA output, deduplicate objects that appear
    identically in more than one database into a single operation each (see
    README's "OpenAPI mapping convention" for why -- there won't be many
    same-path collisions, and where they exist they're the same system
    object, not a real conflict), and drop extended stored procedures this
    pipeline found no way to document at all. Returns
    (operations_not_yet_ranked, extended_procs_dropped_count).
    """
    registry: dict[tuple[str, str, str], dict] = {}

    for db in DATABASES:
        objects = load_json_dump(data_dir / f"{db}.objects.json")
        params = load_json_dump(data_dir / f"{db}.params.json")
        resultsets = load_json_dump(data_dir / f"{db}.resultset.json")
        fmtonly = load_fmtonly_dump(data_dir / f"{db}.resultset_fmtonly.txt")

        params_by_object: dict[tuple[str, str], list[dict]] = {}
        for p in params:
            params_by_object.setdefault((p["schema_name"], p["object_name"]), []).append(p)

        resultset_by_object: dict[tuple[str, str], list[dict]] = {}
        for r in resultsets:
            resultset_by_object.setdefault((r["schema_name"], r["object_name"]), []).append(r)

        for obj in objects:
            key = (obj["schema_name"], obj["object_name"], obj["object_type_desc"])
            entry = registry.setdefault(key, {"object": obj, "databases": [], "candidates": {}})
            entry["databases"].append(db)
            object_key = (obj["schema_name"], obj["object_name"])
            entry["candidates"][db] = {
                "params": params_by_object.get(object_key, []),
                "resultset_rows": resultset_by_object.get(object_key),
                "fmtonly": fmtonly.get(object_key),
            }

    operations = []
    extended_dropped = 0
    for (schema_name, name, type_desc), entry in registry.items():
        databases = entry["databases"]
        params = first_nonempty([entry["candidates"][db]["params"] for db in DATABASES if db in entry["candidates"]])
        resultset = choose_best_resultset(entry["candidates"])

        has_columns = resultset["tier"] in ("described", "fmtonly")
        if type_desc == "EXTENDED_STORED_PROCEDURE" and not params and not has_columns:
            # No sys.all_parameters rows (extended procs are opaque compiled
            # DLLs, not catalogued like a T-SQL/CLR object) and neither
            # introspection method recovered any columns either -- there is
            # nothing real to document, so this operation is left out
            # entirely rather than emitted parameterless/columnless, per the
            # brief for this rewrite.
            extended_dropped += 1
            continue

        operations.append({
            "schema_name": schema_name,
            "name": name,
            "type_desc": type_desc,
            "databases": sorted(databases),
            "params": params,
            "resultset": resultset,
            "has_params": bool([p for p in params if not p.get("is_output")]),
            "has_columns": has_columns,
        })

    return operations, extended_dropped


def rank_key(op: dict) -> tuple:
    """Primary: cross-version presence (see compute_presence()) -- an
    operation that exists on every extracted version ranks ahead of one that
    only exists on some, so the top-{TOP_N_PER_VERSION} cut is as similar as
    possible across versions and prompts/workflows built against it keep
    working regardless of which version is active. Secondary: metadata
    completeness (has both real params and real columns ranks best, neither
    ranks worst). Tertiary: object-type tier (procs > functions > views).
    Quaternary: alphabetical, for a stable/reviewable ordering among
    equally-ranked operations.
    """
    presence_rank = -op.get("presence_count", 1)
    if op["has_params"] and op["has_columns"]:
        completeness = 0
    elif op["has_columns"]:
        completeness = 1
    elif op["has_params"]:
        completeness = 2
    else:
        completeness = 3
    type_tier = _TYPE_TIER.get(op["type_desc"], 3)
    return (presence_rank, completeness, type_tier, op["schema_name"], op["name"])


def main() -> None:
    if len(sys.argv) != 2:
        print(__doc__)
        sys.exit(1)
    version = sys.argv[1]

    data_dir = ROOT / "data" / version
    operations, extended_dropped = build_operations(version, data_dir)

    presence, versions_scanned = compute_presence(ROOT / "data")
    for op in operations:
        key = (op["schema_name"], op["name"], op["type_desc"])
        op["presence_count"] = len(presence.get(key, {version}))

    operations.sort(key=rank_key)
    total_ranked = len(operations)
    kept = operations[:TOP_N_PER_VERSION]
    dropped_by_rank = total_ranked - len(kept)

    paths: dict = {}
    schemas: dict = {SQL_ERROR_SCHEMA_NAME: SQL_ERROR_SCHEMA}
    error_responses = build_error_responses()
    path_collisions = 0

    for op in kept:
        schema_name, name, type_desc = op["schema_name"], op["name"], op["type_desc"]
        path = f"/{schema_name}/{name}"
        if path in paths:
            # Same schema+name but a different object_type_desc across
            # databases (e.g. a view in one database, a proc of the same
            # name in another) -- vanishingly rare for system-shipped
            # objects, but paths/operationIds only encode schema+name, so a
            # second entry here would silently clobber the first. Keep
            # whichever sorted earlier (already the better-ranked one, since
            # `kept` is rank-ordered) and drop this one instead of losing an
            # operation to a silent dict overwrite.
            path_collisions += 1
            continue
        op_id = f"{schema_name}_{name}"

        request_schema = build_request_schema(op["params"], op["databases"])
        output_param_schema = build_output_param_schema(op["params"])
        response_schema = build_response_schema(op["resultset"])

        operation: dict = {
            "operationId": op_id,
            "summary": object_summary(name, type_desc),
            "tags": [type_desc],
            # Redundant with `tags`, but `description` is (a) a standard
            # OpenAPI *operation*-level field, unlike `tags` which mcpify's
            # generated store drops entirely, and (b) otherwise unused here
            # (every operation only ever sets `summary`) -- so a downstream
            # consumer of the generated store's `description` column
            # (mcpify never persists `tags`, only `summary`/`description`)
            # can recover exactly what kind of object this is (VIEW vs.
            # SQL_STORED_PROCEDURE vs. SQL_INLINE_TABLE_VALUED_FUNCTION,
            # etc.) without brittle keyword-matching against `summary`'s
            # free-text wording. See docs/sqlserver-eda-openapi-pipeline
            # README's "OpenAPI mapping convention".
            "description": type_desc,
            # Redundant with build_request_schema()'s EXECUTION_DATABASE_PARAM
            # description, but explicit fields save tooling from having to
            # parse prose -- same rationale as x-sql-type. Unlike the old
            # per-database x-sql-database (singular), this is now a list:
            # this operation is documented once, and may exist identically
            # in more than one of master/msdb/model.
            "x-sql-databases": op["databases"],
            "x-sql-schema": schema_name,
            "responses": {
                "200": {
                    "description": "Result set (if any) returned by the object.",
                    "content": {"application/json": {"schema": response_schema}},
                },
                # deep-copied per operation: reusing the same nested dict
                # object across every operation would make PyYAML emit
                # &anchor/*alias syntax for the repeated reference instead of
                # writing the content out in full each time.
                **copy.deepcopy(error_responses),
            },
        }

        # Every operation now has a requestBody (the synthetic
        # EXECUTION_DATABASE_PARAM guarantees request_schema is never empty)
        # -- see build_request_schema()'s doc comment.
        schema_key = f"{op_id}_Request"
        schemas[schema_key] = request_schema
        operation["requestBody"] = {
            "content": {"application/json": {"schema": {"$ref": f"#/components/schemas/{schema_key}"}}}
        }

        if output_param_schema is not None:
            schema_key = f"{op_id}_OutputParams"
            schemas[schema_key] = output_param_schema
            operation["responses"]["200"]["headers"] = {
                "X-Output-Parameters-Schema": {
                    "description": "Shape of this object's OUTPUT parameters (returned out-of-band by real callers).",
                    "schema": {"$ref": f"#/components/schemas/{schema_key}"},
                }
            }

        paths[path] = {"post": operation}

    security_schemes, security = build_security(version)

    doc = {
        "openapi": "3.1.0",
        "info": {
            "title": f"SQL Server {version} - master/msdb/model combined catalog",
            "version": str(version),
            "description": (
                f"Synthetic OpenAPI representation of system-shipped (is_ms_shipped = 1) stored "
                f"procedures, functions, and views found across the 'master', 'msdb', and 'model' "
                f"databases on SQL Server {version}, generated by introspecting a live instance and "
                f"deduplicating objects that appear identically in more than one of those databases "
                f"(see each operation's `x-sql-databases`). Ranked primarily by how many extracted "
                f"SQL Server versions (2017/2019/2022/2025) an operation exists on -- so the "
                f"top-{TOP_N_PER_VERSION} cut lands on roughly the same operation set across "
                f"versions -- then by metadata completeness; see tools/generate_openapi.py's "
                f"rank_key()/compute_presence(). Each path is a synthetic POST operation (SQL objects are not HTTP "
                f"resources) -- see README for the mapping convention and known limitations. Every "
                f"operation carries an optional `{EXECUTION_DATABASE_PARAM}` request property "
                f"documenting which database to run it against; `security` lists the TDS-protocol "
                f"authentication modes this engine version accepts (mapped to the closest-fitting "
                f"OpenAPI securityScheme shape, not a real HTTP auth flow)."
            ),
        },
        "paths": paths,
        "components": {"schemas": schemas, "securitySchemes": security_schemes},
        "security": security,
    }

    out_dir = ROOT / "openapi" / version
    out_dir.mkdir(parents=True, exist_ok=True)
    out_path = out_dir / "combined.yaml"
    with out_path.open("w", encoding="utf-8") as f:
        yaml.dump(doc, f, sort_keys=False, allow_unicode=True, width=100)

    print(
        f"wrote {out_path} ({len(paths)} operations, {len(schemas)} schemas) -- "
        f"{total_ranked} candidates after dedup, {dropped_by_rank} ranked out beyond top "
        f"{TOP_N_PER_VERSION}, {extended_dropped} extended stored procedures dropped "
        f"(no introspectable params/columns), {path_collisions} path collisions dropped, "
        f"ranked against {versions_scanned} version(s) of data/ present for cross-version "
        f"presence (see compute_presence())"
    )


if __name__ == "__main__":
    main()
