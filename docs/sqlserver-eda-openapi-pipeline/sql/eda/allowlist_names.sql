/*
 * Shared :r-included fragment: populates #allowlist_names (already created by
 * the including script). Keep in sync with ../eda/allowlist.yaml.
 */
INSERT INTO #allowlist_names (name) VALUES
    ('sp_who'),('sp_who2'),('sp_help'),('sp_helpdb'),('sp_helptext'),('sp_helpindex'),
    ('sp_helpconstraint'),('sp_columns'),('sp_tables'),('sp_stored_procedures'),
    ('sp_databases'),('sp_server_info'),('sp_configure'),('sp_rename'),
    ('sp_executesql'),('sp_execute'),('sp_prepare'),('sp_unprepare'),
    ('sp_addlinkedserver'),('sp_droplinkedserver'),('sp_linkedservers'),
    ('sp_addrole'),('sp_addrolemember'),('sp_addlogin'),('sp_grantdbaccess'),
    ('sp_depends'),('sp_lock'),('sp_monitor'),('sp_spaceused'),
    ('sp_estimate_data_compression_savings'),('sp_set_session_context'),
    ('sp_describe_first_result_set'),('sp_describe_undeclared_parameters'),
    ('sp_msforeachtable'),('sp_msforeachdb'),
    ('sp_add_job'),('sp_add_jobstep'),('sp_add_jobschedule'),('sp_add_schedule'),
    ('sp_start_job'),('sp_stop_job'),('sp_delete_job'),('sp_help_job'),
    ('sp_help_jobstep'),('sp_help_jobschedule'),('sp_help_schedule'),('sp_helphistory'),
    ('objects'),('all_objects'),('system_objects'),('procedures'),('parameters'),
    ('all_parameters'),('system_parameters'),('columns'),('tables'),('views'),
    ('types'),('schemas'),('databases'),('indexes'),('index_columns'),
    ('foreign_keys'),('check_constraints'),('sql_modules'),('triggers'),
    ('server_principals'),('database_principals'),('extended_properties'),
    ('TABLES'),('COLUMNS'),('VIEWS'),('ROUTINES'),('PARAMETERS'),
    ('KEY_COLUMN_USAGE'),('TABLE_CONSTRAINTS'),('REFERENTIAL_CONSTRAINTS'),
    ('CHECK_CONSTRAINTS'),('SCHEMATA');
