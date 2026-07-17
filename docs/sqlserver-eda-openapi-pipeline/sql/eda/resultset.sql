/*
 * resultset.sql
 *
 * Attempts to describe the output shape of each matched procedure using
 * sys.dm_exec_describe_first_result_set, and of each matched view/function
 * using sys.columns / sys.dm_exec_describe_first_result_set as applicable.
 *
 * Many objects will fail introspection (extended procs, procs with
 * conditional result sets that need specific arguments, procs that require
 * elevated permissions or special session state). Those are recorded with
 * result_set_status = 'unknown' rather than guessed at -- see README
 * limitations.
 *
 * Run the same way as objects.sql, once per database per version:
 *   sqlcmd -S localhost,<port> -U sa -P "$MSSQL_SA_PASSWORD" -C \
 *     -v db=<master|msdb|sandbox> -i sql/eda/resultset.sql -o data/<version>/<db>.resultset.json
 */

-- See objects.sql for why this is an explicit USE off a required sqlcmd
-- scripting variable rather than relying on sqlcmd's -d connection flag.
-- It matters even more here than in objects.sql/params.sql:
-- sys.dm_exec_describe_first_result_set(@sql, ...) below resolves unqualified
-- object names in @sql against whatever database is *current* on the
-- connection -- if that's silently wrong, every EXEC/SELECT it builds
-- resolves against the wrong database's objects instead of failing loudly.
USE $(db);

SET NOCOUNT ON;

IF OBJECT_ID('tempdb..#allowlist_names') IS NOT NULL DROP TABLE #allowlist_names;
CREATE TABLE #allowlist_names (name sysname NOT NULL);
:r allowlist_names.sql

IF OBJECT_ID('tempdb..#allowlist_patterns') IS NOT NULL DROP TABLE #allowlist_patterns;
CREATE TABLE #allowlist_patterns (pattern nvarchar(200) NOT NULL);
:r allowlist_patterns.sql

IF OBJECT_ID('tempdb..#targets') IS NOT NULL DROP TABLE #targets;
SELECT
    o.object_id,
    SCHEMA_NAME(o.schema_id) AS schema_name,
    o.name                    AS object_name,
    o.type                    AS object_type_code
INTO #targets
FROM sys.all_objects AS o
WHERE (EXISTS (SELECT 1 FROM #allowlist_names n WHERE n.name = o.name)
    OR EXISTS (SELECT 1 FROM #allowlist_patterns p WHERE o.name LIKE p.pattern))
  AND o.type IN ('P', 'PC', 'V');  -- procedures, CLR procs, views (result-set introspectable via T-SQL EXEC/SELECT)

IF OBJECT_ID('tempdb..#results') IS NOT NULL DROP TABLE #results;
CREATE TABLE #results (
    schema_name      sysname,
    object_name      sysname,
    object_type_code nchar(2),
    result_set_status varchar(20),
    error_message     nvarchar(2000) NULL,
    column_ordinal    int NULL,
    column_name       sysname NULL,
    system_type_name  nvarchar(256) NULL,
    is_nullable       bit NULL
);

DECLARE @schema sysname, @name sysname, @type nchar(2), @sql nvarchar(max);

DECLARE target_cursor CURSOR LOCAL FAST_FORWARD FOR
    SELECT schema_name, object_name, object_type_code FROM #targets;

OPEN target_cursor;
FETCH NEXT FROM target_cursor INTO @schema, @name, @type;

WHILE @@FETCH_STATUS = 0
BEGIN
    BEGIN TRY
        IF @type = 'V'
            SET @sql = N'SELECT * FROM ' + QUOTENAME(@schema) + N'.' + QUOTENAME(@name);
        ELSE
            SET @sql = N'EXEC ' + QUOTENAME(@schema) + N'.' + QUOTENAME(@name);

        INSERT INTO #results (schema_name, object_name, object_type_code, result_set_status,
                               column_ordinal, column_name, system_type_name, is_nullable)
        SELECT @schema, @name, @type, 'described',
               column_ordinal, name, system_type_name, is_nullable
        FROM sys.dm_exec_describe_first_result_set(@sql, NULL, 0);

        IF @@ROWCOUNT = 0
            INSERT INTO #results (schema_name, object_name, object_type_code, result_set_status)
            VALUES (@schema, @name, @type, 'no_result_set');
    END TRY
    BEGIN CATCH
        INSERT INTO #results (schema_name, object_name, object_type_code, result_set_status, error_message)
        VALUES (@schema, @name, @type, 'unknown', ERROR_MESSAGE());
    END CATCH

    FETCH NEXT FROM target_cursor INTO @schema, @name, @type;
END

CLOSE target_cursor;
DEALLOCATE target_cursor;

SELECT * FROM #results
ORDER BY schema_name, object_name, column_ordinal
FOR JSON PATH;
