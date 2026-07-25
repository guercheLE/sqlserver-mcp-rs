/*
 * resultset.sql
 *
 * Best-effort, execution-free result-set introspection for procedures,
 * functions, and views (types 'P', 'FN', 'IF', 'TF', 'V' -- NOT 'PC'/'FS'/
 * 'FT'/'X', see below) objects.sql would match, via
 * sys.dm_exec_describe_first_result_set -- this performs static analysis of
 * the T-SQL text it's given and, per Microsoft, never executes the
 * statement, so it's safe to attempt even on mutating procedures
 * (sp_rename, sp_delete_job, ...).
 *
 * CLR objects ('PC'/'FS'/'FT') and extended stored procedures ('X') are
 * deliberately excluded from this script entirely, not just left to fail
 * via TRY/CATCH: there is no T-SQL AST for describe_first_result_set to
 * statically analyze for compiled-DLL/.NET-assembly code, so determining
 * their result shape this way apparently still touches native/managed
 * code paths -- confirmed live against a SQL Server 2017 container during
 * this pipeline's development, where describing one such object raised a
 * severity-20 fatal exception that tore down the whole sqlcmd session
 * (severity 20+ errors terminate the connection and bypass TRY/CATCH
 * entirely -- there is no way to recover from one mid-script). Excluding
 * these types is the only reliable mitigation; resultset_fmtonly.sql
 * already excluded 'X' for a related but distinct safety reason (its "no
 * data altered" guarantee doesn't cover compiled-DLL code either) and now
 * excludes 'PC'/'FS'/'FT' too for this same crash risk.
 *
 * Since most system procs/functions have at least one required parameter,
 * a bare `EXEC schema.name` often isn't enough for the query processor to
 * resolve the call -- this builds a call with every declared parameter
 * explicitly passed as NULL (named `@param = NULL` for procedures, so
 * argument order never matters; positional for functions, which have no
 * named-call syntax) so describe_first_result_set has a syntactically
 * complete statement to analyze. Many objects still won't describe --
 * extended procs, procs with conditional/dynamic result sets, procs needing
 * elevated permissions or a specific session state. Those are recorded with
 * result_set_status = 'unknown' rather than guessed at; tools/
 * generate_openapi.py then falls back to resultset_fmtonly.sql's output for
 * those before giving up on columns entirely -- see README limitations.
 *
 * Run the same way as objects.sql, once per database per version:
 *   sqlcmd -S localhost,<port> -U sa -P "$MSSQL_SA_PASSWORD" -C \
 *     -v db=<master|msdb|model> -i sql/eda/resultset.sql -o data/<version>/<db>.resultset.json
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

IF OBJECT_ID('tempdb..#targets') IS NOT NULL DROP TABLE #targets;
SELECT
    o.object_id,
    SCHEMA_NAME(o.schema_id) AS schema_name,
    o.name                    AS object_name,
    o.type                    AS object_type_code
INTO #targets
FROM sys.all_objects AS o
WHERE o.is_ms_shipped = 1
  AND o.type IN ('P', 'FN', 'IF', 'TF', 'V');  -- 'PC'/'FS'/'FT'/'X' deliberately excluded -- see header comment

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

DECLARE @schema sysname, @name sysname, @type nchar(2), @object_id int, @sql nvarchar(max), @args nvarchar(max);

DECLARE target_cursor CURSOR LOCAL FAST_FORWARD FOR
    SELECT schema_name, object_name, object_type_code, object_id FROM #targets;

OPEN target_cursor;
FETCH NEXT FROM target_cursor INTO @schema, @name, @type, @object_id;

WHILE @@FETCH_STATUS = 0
BEGIN
    BEGIN TRY
        IF @type = 'V'
        BEGIN
            SET @sql = N'SELECT * FROM ' + QUOTENAME(@schema) + N'.' + QUOTENAME(@name);
        END
        ELSE IF @type IN ('FN', 'FS')
        BEGIN
            -- Scalar functions: positional NULLs (no named-argument call
            -- syntax exists for a bare function call), wrapped as a single
            -- aliased column so the "result set" has a stable column name.
            SELECT @args = STRING_AGG(CAST(N'NULL' AS nvarchar(max)), N', ') WITHIN GROUP (ORDER BY parameter_id)
            FROM sys.all_parameters WHERE object_id = @object_id AND parameter_id > 0;
            SET @sql = N'SELECT ' + QUOTENAME(@schema) + N'.' + QUOTENAME(@name)
                     + N'(' + ISNULL(@args, N'') + N') AS result';
        END
        ELSE IF @type IN ('IF', 'TF', 'FT')
        BEGIN
            -- Table-valued functions: positional NULLs in the FROM-clause call.
            SELECT @args = STRING_AGG(CAST(N'NULL' AS nvarchar(max)), N', ') WITHIN GROUP (ORDER BY parameter_id)
            FROM sys.all_parameters WHERE object_id = @object_id AND parameter_id > 0;
            SET @sql = N'SELECT * FROM ' + QUOTENAME(@schema) + N'.' + QUOTENAME(@name)
                     + N'(' + ISNULL(@args, N'') + N')';
        END
        ELSE
        BEGIN
            -- Procedures (P/PC/X): named `@param = NULL` so declared
            -- parameter order never has to match call order.
            SELECT @args = STRING_AGG(CAST(N'@' + name + N' = NULL' AS nvarchar(max)), N', ') WITHIN GROUP (ORDER BY parameter_id)
            FROM sys.all_parameters WHERE object_id = @object_id AND parameter_id > 0 AND name IS NOT NULL;
            SET @sql = N'EXEC ' + QUOTENAME(@schema) + N'.' + QUOTENAME(@name)
                     + CASE WHEN @args IS NULL THEN N'' ELSE N' ' + @args END;
        END

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

    FETCH NEXT FROM target_cursor INTO @schema, @name, @type, @object_id;
END

CLOSE target_cursor;
DEALLOCATE target_cursor;

SELECT * FROM #results
ORDER BY schema_name, object_name, column_ordinal
FOR JSON PATH;
