#!/usr/bin/env python3
"""Turn EDA JSON dumps (data/<version>/<db>.{objects,params,resultset}.json)
into synthetic OpenAPI 3.1 YAML (openapi/<version>/<db>.yaml), one POST
operation per documented stored procedure/function/view.

Usage:
    tools/generate_openapi.py <version> <database>
    tools/generate_openapi.py 2022 master
"""
from __future__ import annotations

import copy
import json
import sys
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parent.parent

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
    raw = path.read_text(encoding="utf-8-sig")
    text = raw.replace("\r\n", "\n").replace("\n", "").strip()
    if not text:
        return []
    start = min((i for i in (text.find("["), text.find("{")) if i != -1), default=-1)
    if start == -1:
        raise ValueError(f"{path}: no JSON array/object found in sqlcmd output")
    return json.loads(text[start:])


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

# Hand-curated one-line summaries for the well-known objects in the allowlist.
# System object metadata has no reliable description field to pull this from
# automatically (see README limitations), so this is maintained by hand.
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

# Hand-curated parameter signatures for objects that live in sys.all_objects
# (so objects.sql matches them) but have ZERO rows in sys.all_parameters --
# confirmed live against a running SQL Server 2022 instance, and confirmed
# that sp_help doesn't recover them either (sp_help reads the same catalog).
# All seven are EXTENDED_STORED_PROCEDURE internally: their calling
# convention is hardcoded into the query processor rather than catalogued
# like a regular T-SQL/CLR proc's parameters, so this is the only way to
# document their signatures -- same treatment already used for xp_* procs,
# extended to these because they're common/important enough to be worth it.
# Verified against Microsoft Learn (fetched 2026-07-17):
#   sp_executesql:                  learn.microsoft.com/sql/relational-databases/system-stored-procedures/sp-executesql-transact-sql
#   sp_prepare:                     learn.microsoft.com/sql/relational-databases/system-stored-procedures/sp-prepare-transact-sql
#   sp_execute:                     learn.microsoft.com/sql/relational-databases/system-stored-procedures/sp-execute-transact-sql
#   sp_unprepare:                   learn.microsoft.com/sql/relational-databases/system-stored-procedures/sp-unprepare-transact-sql
#   sp_describe_first_result_set:   learn.microsoft.com/sql/relational-databases/system-stored-procedures/sp-describe-first-result-set-transact-sql
#   sp_describe_undeclared_parameters: learn.microsoft.com/sql/relational-databases/system-stored-procedures/sp-describe-undeclared-parameters-transact-sql
#   sp_set_session_context:         learn.microsoft.com/sql/relational-databases/system-stored-procedures/sp-set-session-context-transact-sql
#
# Two of these docs are internally inconsistent, resolved here by following
# the worked examples over the conflicting prose/syntax box:
#   - sp_prepare's argument prose calls `params` "a required OUTPUT
#     parameter", but its own example passes it as a plain literal
#     (N'@P1 NVARCHAR(128), ...') with no OUTPUT keyword -- treated as input.
#   - sp_execute's syntax box shows "handle OUTPUT", but the argument
#     description and worked example (`EXECUTE sp_execute 1, 49879;`, no
#     OUTPUT keyword) both treat it as a plain input -- treated as input.
CURATED_PARAMETERS = {
    "sp_executesql": {
        "params": [
            {"parameter_name": "@stmt", "ordinal": 1, "data_type": "nvarchar", "max_length": -1,
             "precision": 0, "scale": 0, "is_output": False, "has_default_value": False, "default_value": None},
            {"parameter_name": "@params", "ordinal": 2, "data_type": "nvarchar", "max_length": -1,
             "precision": 0, "scale": 0, "is_output": False, "has_default_value": True, "default_value": None},
        ],
        "note": (
            "Additional @param1..@paramN values (and OUT/OUTPUT-flagged ones) are declared "
            "dynamically by the @params string at call time and can't be enumerated statically."
        ),
    },
    "sp_prepare": {
        "params": [
            {"parameter_name": "@handle", "ordinal": 1, "data_type": "int", "max_length": 4,
             "precision": 10, "scale": 0, "is_output": True, "has_default_value": False, "default_value": None},
            {"parameter_name": "@params", "ordinal": 2, "data_type": "nvarchar", "max_length": -1,
             "precision": 0, "scale": 0, "is_output": False, "has_default_value": False, "default_value": None},
            {"parameter_name": "@stmt", "ordinal": 3, "data_type": "nvarchar", "max_length": -1,
             "precision": 0, "scale": 0, "is_output": False, "has_default_value": False, "default_value": None},
            {"parameter_name": "@options", "ordinal": 4, "data_type": "int", "max_length": 4,
             "precision": 10, "scale": 0, "is_output": False, "has_default_value": True, "default_value": 0},
        ],
        "note": (
            "@handle is returned by the engine (OUTPUT) for use with sp_execute/sp_unprepare. "
            "@options is a bitmask; only 0x0001 (RETURN_METADATA) is documented."
        ),
    },
    "sp_execute": {
        "params": [
            {"parameter_name": "@handle", "ordinal": 1, "data_type": "int", "max_length": 4,
             "precision": 10, "scale": 0, "is_output": False, "has_default_value": False, "default_value": None},
        ],
        "note": (
            "Additional positional/named bound_param values must match the declarations made by "
            "the sp_prepare @params string and can't be enumerated statically."
        ),
    },
    "sp_unprepare": {
        "params": [
            {"parameter_name": "@handle", "ordinal": 1, "data_type": "int", "max_length": 4,
             "precision": 10, "scale": 0, "is_output": False, "has_default_value": False, "default_value": None},
        ],
    },
    "sp_describe_first_result_set": {
        "params": [
            {"parameter_name": "@tsql", "ordinal": 1, "data_type": "nvarchar", "max_length": -1,
             "precision": 0, "scale": 0, "is_output": False, "has_default_value": False, "default_value": None},
            {"parameter_name": "@params", "ordinal": 2, "data_type": "nvarchar", "max_length": -1,
             "precision": 0, "scale": 0, "is_output": False, "has_default_value": True, "default_value": None},
            {"parameter_name": "@browse_information_mode", "ordinal": 3, "data_type": "tinyint", "max_length": 1,
             "precision": 3, "scale": 0, "is_output": False, "has_default_value": True, "default_value": 0},
        ],
        "note": "@browse_information_mode: 0 = none, 1 = FOR BROWSE-style, 2 = cursor-preparation-style.",
    },
    "sp_describe_undeclared_parameters": {
        "params": [
            {"parameter_name": "@tsql", "ordinal": 1, "data_type": "nvarchar", "max_length": -1,
             "precision": 0, "scale": 0, "is_output": False, "has_default_value": False, "default_value": None},
            {"parameter_name": "@params", "ordinal": 2, "data_type": "nvarchar", "max_length": -1,
             "precision": 0, "scale": 0, "is_output": False, "has_default_value": True, "default_value": None},
        ],
    },
    "sp_set_session_context": {
        "params": [
            {"parameter_name": "@key", "ordinal": 1, "data_type": "nvarchar", "max_length": 256,
             "precision": 0, "scale": 0, "is_output": False, "has_default_value": False, "default_value": None},
            {"parameter_name": "@value", "ordinal": 2, "data_type": "sql_variant", "max_length": 8000,
             "precision": 0, "scale": 0, "is_output": False, "has_default_value": False, "default_value": None},
            {"parameter_name": "@read_only", "ordinal": 3, "data_type": "bit", "max_length": 1,
             "precision": 1, "scale": 0, "is_output": False, "has_default_value": True, "default_value": False},
        ],
        "note": "@key: sysname, max 128 bytes. @value: max 8,000 bytes; NULL frees the key's memory.",
    },
}


def object_summary(name: str, type_desc: str) -> str:
    if name in SUMMARIES:
        return SUMMARIES[name]
    if type_desc.startswith("SQL_STORED_PROCEDURE") or type_desc == "CLR_STORED_PROCEDURE":
        return f"System stored procedure {name} (see Microsoft Learn for details)."
    if "TABLE_VALUED_FUNCTION" in type_desc or type_desc.endswith("_FUNCTION"):
        return f"System function {name} (see Microsoft Learn for details)."
    if type_desc == "VIEW":
        return f"System/catalog view {name} (see Microsoft Learn for details)."
    return f"System object {name}."


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


def build_request_schema(params: list[dict]) -> dict | None:
    input_params = [p for p in params if not p.get("is_output")]
    if not input_params:
        return None
    properties = {}
    required = []
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


def build_response_schema(rows: list[dict] | None) -> dict:
    if not rows:
        return {"description": "Result set shape unknown (introspection not attempted or object has no rows)."}
    status = rows[0].get("result_set_status")
    if status == "no_result_set":
        return {"type": "object", "description": "This object does not return a result set."}
    if status == "unknown":
        err = rows[0].get("error_message")
        desc = "Result set shape could not be determined by introspection"
        if err:
            desc += f": {err}"
        return {"type": "object", "description": desc}
    # status == "described"
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


def main() -> None:
    if len(sys.argv) != 3:
        print(__doc__)
        sys.exit(1)
    version, db = sys.argv[1], sys.argv[2]

    data_dir = ROOT / "data" / version
    objects = load_json_dump(data_dir / f"{db}.objects.json")
    params = load_json_dump(data_dir / f"{db}.params.json")
    resultsets = load_json_dump(data_dir / f"{db}.resultset.json")

    params_by_object: dict[tuple[str, str], list[dict]] = {}
    for p in params:
        key = (p["schema_name"], p["object_name"])
        params_by_object.setdefault(key, []).append(p)

    resultset_by_object: dict[tuple[str, str], list[dict]] = {}
    for r in resultsets:
        key = (r["schema_name"], r["object_name"])
        resultset_by_object.setdefault(key, []).append(r)

    paths: dict = {}
    schemas: dict = {SQL_ERROR_SCHEMA_NAME: SQL_ERROR_SCHEMA}
    error_responses = build_error_responses()

    for obj in sorted(objects, key=lambda o: (o["schema_name"], o["object_name"])):
        schema_name = obj["schema_name"]
        name = obj["object_name"]
        type_desc = obj["object_type_desc"]
        key = (schema_name, name)
        op_id = f"{schema_name}_{name}"

        obj_params = params_by_object.get(key, [])
        params_are_curated = False
        if not obj_params and name in CURATED_PARAMETERS:
            obj_params = CURATED_PARAMETERS[name]["params"]
            params_are_curated = True
        request_schema = build_request_schema(obj_params)
        output_param_schema = build_output_param_schema(obj_params)
        response_schema = build_response_schema(resultset_by_object.get(key))

        if params_are_curated:
            source_note = (
                "Hand-curated from Microsoft Learn, not from live introspection -- "
                "sys.all_parameters has no rows for this object (it's an "
                "EXTENDED_STORED_PROCEDURE whose calling convention is hardcoded into "
                "the query processor). See README limitations."
            )
            extra_note = CURATED_PARAMETERS[name].get("note")
            for schema in (request_schema, output_param_schema):
                if schema is not None:
                    schema["x-sql-params-source"] = "hand-curated"
                    schema["description"] = source_note + (f" {extra_note}" if extra_note else "")

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
            # Redundant with the path/operationId, but explicit fields save
            # tooling from having to parse schema/database back out of a
            # string -- same rationale as x-sql-type.
            "x-sql-database": db,
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

        if request_schema is not None:
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

        paths[f"/{schema_name}/{name}"] = {"post": operation}

    security_schemes, security = build_security(version)

    doc = {
        "openapi": "3.1.0",
        "info": {
            "title": f"SQL Server {version} - {db} system object catalog",
            "version": str(version),
            "description": (
                f"Synthetic OpenAPI representation of curated system stored procedures, "
                f"functions, and catalog views in the '{db}' database on SQL Server {version}, "
                f"generated by introspecting a live instance. Each path is a synthetic POST "
                f"operation (SQL objects are not HTTP resources) -- see README for the mapping "
                f"convention and known limitations. `security` lists the TDS-protocol "
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
    out_path = out_dir / f"{db}.yaml"
    with out_path.open("w", encoding="utf-8") as f:
        yaml.dump(doc, f, sort_keys=False, allow_unicode=True, width=100)

    print(f"wrote {out_path} ({len(paths)} operations, {len(schemas)} schemas)")


if __name__ == "__main__":
    main()
