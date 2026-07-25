/*
 * objects.sql
 *
 * Every system-shipped "operation" in the current database: stored
 * procedures (T-SQL, CLR, and extended/DLL), functions (scalar, inline
 * table-valued, multi-statement table-valued, CLR scalar/table-valued), and
 * views. Views cover three things SQL Server all implements the same way --
 * catalog views (sys.objects, ...), INFORMATION_SCHEMA views, and DMVs
 * (sys.dm_exec_*, ...) -- there is no separate "DMV" object type to filter
 * on; they're all type 'V'.
 *
 * is_ms_shipped = 1 keeps this to objects the engine itself ships, not
 * anything a user or tool happened to create in these databases (there is
 * no curated allowlist anymore -- this is a full sweep of the system
 * catalog, deduplicated and ranked downstream by tools/generate_openapi.py).
 *
 * Run via:
 *   sqlcmd -S localhost,<port> -U sa -P "$MSSQL_SA_PASSWORD" -C \
 *     -v db=<master|msdb|model> -i sql/eda/objects.sql -o data/<version>/<db>.objects.json -y 0 -Y 0
 */

-- Which database this runs against (master/msdb/model/or any other) is
-- driven explicitly by this USE, not implicitly by sqlcmd's -d connection
-- flag -- $(db) is a required sqlcmd scripting variable (passed with
-- `-v db=<name>`; scripts/extract.sh and scripts/diff_versions.sh already do
-- this). Running this file without -v db=... fails fast rather than
-- silently querying whatever database the connection happened to default to.
USE $(db);

SET NOCOUNT ON;

SELECT
    @@VERSION                 AS sql_version,
    DB_NAME()                  AS database_name,
    o.name                      AS object_name,
    SCHEMA_NAME(o.schema_id)    AS schema_name,
    o.type                       AS object_type_code,
    o.type_desc                  AS object_type_desc,
    o.object_id                   AS object_id,
    o.is_ms_shipped                AS is_ms_shipped
FROM sys.all_objects AS o
WHERE o.is_ms_shipped = 1
  AND o.type IN ('P', 'PC', 'X', 'FN', 'IF', 'TF', 'FS', 'FT', 'V')
FOR JSON PATH;
