/*
 * objects.sql
 *
 * Lists curated objects (see ../eda/allowlist.yaml) that actually exist in the
 * current database, across sys.system_objects and sys.all_objects (functions
 * are user-schema in sys.all_objects even when "system" in behavior, so both
 * are searched). Absence of a name here for a given version/database is
 * itself meaningful -- it means that version doesn't ship the object.
 *
 * Run via:
 *   sqlcmd -S localhost,<port> -U sa -P "$MSSQL_SA_PASSWORD" -C \
 *     -v db=<master|msdb|sandbox> -i sql/eda/objects.sql -o data/<version>/<db>.objects.json -y 0 -Y 0
 *
 * Keep the #allowlist_names / #allowlist_patterns contents in sync with
 * sql/eda/allowlist.yaml -- this SQL copy exists because sqlcmd has no YAML
 * support; allowlist.yaml remains the human-readable source of truth.
 */

-- Which database this runs against (master/msdb/sandbox/or any other) is
-- driven explicitly by this USE, not implicitly by sqlcmd's -d connection
-- flag -- $(db) is a required sqlcmd scripting variable (passed with
-- `-v db=<name>`; scripts/extract.sh and scripts/diff_versions.sh already do
-- this). Running this file without -v db=... fails fast rather than
-- silently querying whatever database the connection happened to default to.
USE $(db);

SET NOCOUNT ON;

IF OBJECT_ID('tempdb..#allowlist_names') IS NOT NULL DROP TABLE #allowlist_names;
CREATE TABLE #allowlist_names (name sysname NOT NULL);
:r allowlist_names.sql

IF OBJECT_ID('tempdb..#allowlist_patterns') IS NOT NULL DROP TABLE #allowlist_patterns;
CREATE TABLE #allowlist_patterns (pattern nvarchar(200) NOT NULL);
:r allowlist_patterns.sql

SELECT
    @@VERSION            AS sql_version,
    DB_NAME()             AS database_name,
    o.name                AS object_name,
    SCHEMA_NAME(o.schema_id) AS schema_name,
    o.type                AS object_type_code,
    o.type_desc           AS object_type_desc,
    o.object_id            AS object_id,
    o.is_ms_shipped        AS is_ms_shipped
FROM sys.all_objects AS o
WHERE EXISTS (SELECT 1 FROM #allowlist_names n WHERE n.name = o.name)
   OR EXISTS (SELECT 1 FROM #allowlist_patterns p WHERE o.name LIKE p.pattern)
FOR JSON PATH;
