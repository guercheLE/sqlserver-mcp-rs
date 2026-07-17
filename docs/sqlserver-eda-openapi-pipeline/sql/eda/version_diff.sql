/*
 * version_diff.sql
 *
 * Same allowlist match as objects.sql, but emits one plain-text line per
 * matched object (schema.name<TAB>type) instead of JSON, specifically so the
 * output from different SQL Server versions can be diffed directly with
 * `diff`/`comm` to see which objects were added/removed/renamed between
 * versions. Run identically against every version+database, then compare:
 *
 *   for v in 2017 2019 2022 2025; do
 *     sqlcmd -S localhost,<port-for-$v> -U sa -P "$MSSQL_SA_PASSWORD" -C \
 *       -v db=master -i sql/eda/version_diff.sql -o data/$v/master.objects.txt -h -1 -W
 *   done
 *   diff data/2019/master.objects.txt data/2022/master.objects.txt
 *
 * (scripts/diff_versions.sh automates the loop + pairwise diffs.)
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

SELECT SCHEMA_NAME(o.schema_id) + N'.' + o.name + CHAR(9) + o.type_desc AS line
FROM sys.all_objects AS o
WHERE EXISTS (SELECT 1 FROM #allowlist_names n WHERE n.name = o.name)
   OR EXISTS (SELECT 1 FROM #allowlist_patterns p WHERE o.name LIKE p.pattern)
ORDER BY line;
