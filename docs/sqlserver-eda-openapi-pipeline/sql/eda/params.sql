/*
 * params.sql
 *
 * Pulls parameter metadata (name, type, direction, default, is_output) for
 * every object matched by objects.sql, from sys.all_parameters (covers both
 * user and system objects; sys.system_parameters is a subset view over the
 * same rows for system objects and is not needed separately).
 *
 * Run the same way as objects.sql, once per database per version:
 *   sqlcmd -S localhost,<port> -U sa -P "$MSSQL_SA_PASSWORD" -C \
 *     -v db=<master|msdb|sandbox> -i sql/eda/params.sql -o data/<version>/<db>.params.json
 */

-- See objects.sql for why this is an explicit USE off a required sqlcmd
-- scripting variable rather than relying on sqlcmd's -d connection flag.
USE $(db);

SET NOCOUNT ON;

IF OBJECT_ID('tempdb..#allowlist_names') IS NOT NULL DROP TABLE #allowlist_names;
CREATE TABLE #allowlist_names (name sysname NOT NULL);
:r allowlist_names.sql

IF OBJECT_ID('tempdb..#allowlist_patterns') IS NOT NULL DROP TABLE #allowlist_patterns;
CREATE TABLE #allowlist_patterns (pattern nvarchar(200) NOT NULL);
:r allowlist_patterns.sql

SELECT
    DB_NAME()                              AS database_name,
    SCHEMA_NAME(o.schema_id)                AS schema_name,
    o.name                                  AS object_name,
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
WHERE EXISTS (SELECT 1 FROM #allowlist_names n WHERE n.name = o.name)
   OR EXISTS (SELECT 1 FROM #allowlist_patterns pat WHERE o.name LIKE pat.pattern)
ORDER BY schema_name, object_name, ordinal
FOR JSON PATH;
