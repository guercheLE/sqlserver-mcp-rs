/*
 * resultset_fmtonly.sql
 *
 * Fallback column-name discovery, via SET FMTONLY ON, for objects whose
 * result set resultset.sql's sys.dm_exec_describe_first_result_set pass
 * couldn't describe. FMTONLY asks the query processor to compile/partially
 * run the statement and report the result set's column metadata over the
 * wire without altering any data -- per Microsoft: "Even if your script
 * includes destructive statements like INSERT, UPDATE, or DELETE, nothing
 * is written to the database." This can succeed in cases the purely static
 * describe_first_result_set pass can't (e.g. temp tables built inside the
 * object's own body). It's deprecated by Microsoft in favor of
 * sp_describe_first_result_set (== resultset.sql's approach) but still
 * functional, and is attempted here specifically as the second-chance path
 * for whatever the first pass already missed.
 *
 * SAFETY: unlike resultset.sql's pass (static analysis, never actually
 * executes anything for pure T-SQL objects), FMTONLY genuinely compiles and
 * partially runs the statement -- Microsoft's own deprecation notice warns
 * it "can occasionally try to execute parts of complex dynamic SQL". Its "no
 * data altered" guarantee is documented for native T-SQL, not for what a
 * CLR (.NET) or extended stored procedure's compiled code might actually do
 * once invoked -- so 'PC'/'FS'/'FT' (CLR) and 'X' (extended, e.g.
 * xp_cmdshell) objects are excluded from this script entirely, never just
 * left to fail: they are never called here at all. resultset.sql excludes
 * the same four types too, for a related but distinct reason -- see that
 * script's header comment for the live crash that made this necessary.
 *
 * THERE IS NO SERVER-SIDE WAY TO CAPTURE FMTONLY'S COLUMN METADATA: it's a
 * wire-protocol response sent to the *client*, not something a T-SQL session
 * can SELECT back out of itself. So this script leans on sqlcmd's own text
 * rendering instead of FOR JSON: each target's FMTONLY-mode statement runs
 * as its own top-level batch, sqlcmd prints that (real, zero-row) result
 * set's column-header line exactly like it would for any query, and a
 * `PRINT '===OBJECT:...==='` marker before each one lets
 * tools/generate_openapi.py split the raw text output back into per-object
 * blocks and read off just the header line. Only column NAMES are
 * recoverable this way -- sqlcmd's text renderer carries no type
 * information for a header row -- so generate_openapi.py tags columns
 * sourced from this script `x-sql-columns-source: fmtonly`, distinct from
 * the fully-typed output of resultset.sql's primary pass.
 *
 * Run via:
 *   sqlcmd -S localhost,<port> -U sa -P "$MSSQL_SA_PASSWORD" -C \
 *     -v db=<master|msdb|model> -i sql/eda/resultset_fmtonly.sql \
 *     -o data/<version>/<db>.resultset_fmtonly.txt -y 0 -Y 0 -w 65535
 */

-- See objects.sql for why this is an explicit USE off a required sqlcmd
-- scripting variable rather than relying on sqlcmd's -d connection flag.
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

DECLARE @schema sysname, @name sysname, @type nchar(2), @object_id int, @sql nvarchar(max), @args nvarchar(max);

DECLARE target_cursor CURSOR LOCAL FAST_FORWARD FOR
    SELECT schema_name, object_name, object_type_code, object_id
    FROM #targets
    ORDER BY schema_name, object_name;

OPEN target_cursor;
FETCH NEXT FROM target_cursor INTO @schema, @name, @type, @object_id;

WHILE @@FETCH_STATUS = 0
BEGIN
    -- This marker is the only thing tying a block of sqlcmd's text output
    -- back to the object it came from -- see the header comment.
    PRINT N'===OBJECT:' + @schema + N'.' + @name + N':' + @type + N'===';

    BEGIN TRY
        IF @type = 'V'
        BEGIN
            SET @sql = N'SELECT * FROM ' + QUOTENAME(@schema) + N'.' + QUOTENAME(@name);
        END
        ELSE IF @type IN ('FN', 'FS')
        BEGIN
            SELECT @args = STRING_AGG(CAST(N'NULL' AS nvarchar(max)), N', ') WITHIN GROUP (ORDER BY parameter_id)
            FROM sys.all_parameters WHERE object_id = @object_id AND parameter_id > 0;
            SET @sql = N'SELECT ' + QUOTENAME(@schema) + N'.' + QUOTENAME(@name)
                     + N'(' + ISNULL(@args, N'') + N') AS result';
        END
        ELSE IF @type IN ('IF', 'TF', 'FT')
        BEGIN
            SELECT @args = STRING_AGG(CAST(N'NULL' AS nvarchar(max)), N', ') WITHIN GROUP (ORDER BY parameter_id)
            FROM sys.all_parameters WHERE object_id = @object_id AND parameter_id > 0;
            SET @sql = N'SELECT * FROM ' + QUOTENAME(@schema) + N'.' + QUOTENAME(@name)
                     + N'(' + ISNULL(@args, N'') + N')';
        END
        ELSE
        BEGIN
            SELECT @args = STRING_AGG(CAST(N'@' + name + N' = NULL' AS nvarchar(max)), N', ') WITHIN GROUP (ORDER BY parameter_id)
            FROM sys.all_parameters WHERE object_id = @object_id AND parameter_id > 0 AND name IS NOT NULL;
            SET @sql = N'EXEC ' + QUOTENAME(@schema) + N'.' + QUOTENAME(@name)
                     + CASE WHEN @args IS NULL THEN N'' ELSE N' ' + @args END;
        END

        SET FMTONLY ON;
        EXEC (@sql);
        SET FMTONLY OFF;
    END TRY
    BEGIN CATCH
        SET FMTONLY OFF;  -- guaranteed off before the next iteration even if the TRY block errored mid-statement
        PRINT N'ERROR: ' + ERROR_MESSAGE();
    END CATCH

    FETCH NEXT FROM target_cursor INTO @schema, @name, @type, @object_id;
END

CLOSE target_cursor;
DEALLOCATE target_cursor;
