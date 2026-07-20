//! `#[prompt_router]`-decorated `impl McpifyServer` block — one method per
//! MCP prompt. Kept separate from `src/core/mcp_server.rs`'s `#[tool_router]`
//! block (see `docs/mcp-prompts-workflow-plan.md`). `vis = "pub(crate)"` is
//! required here (unlike the co-located `tool_router`) because this impl
//! block lives in a different module than `McpifyServer::new()`, which calls
//! the generated `Self::prompt_router()`.

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{PromptMessage, Role};
use rmcp::{prompt, prompt_router};

use crate::core::mcp_server::McpifyServer;
use crate::prompts::{
    IndexesConstraintsArgs, MasterWorkflowArgs, SecurityProvisioningArgs, SqlAgentJobsArgs,
    render_context_header,
};

#[prompt_router(vis = "pub(crate)")]
impl McpifyServer {
    #[prompt(
        name = "sqlserver_workflow",
        description = "Start here. Presents the available SQL Server operational \
                        workflows, routes to the right guided sub-workflow based on \
                        the user's goal, and — where the environment supports it — \
                        delegates that whole sub-workflow to an isolated sub-task to \
                        spare this conversation's context window."
    )]
    async fn sqlserver_workflow_prompt(
        &self,
        Parameters(args): Parameters<MasterWorkflowArgs>,
    ) -> Vec<PromptMessage> {
        let header = render_context_header(&[("goal", args.goal.as_deref())]);
        vec![PromptMessage::new_text(
            Role::User,
            format!("{header}\n{}", include_str!("content/master.md")),
        )]
    }

    #[prompt(
        name = "sqlserver_workflow_sql_agent_jobs",
        description = "Guided SQL Agent job setup: create a job, add one or more \
                        steps, attach a schedule, and start it — each step gated on \
                        the previous one being confirmed to exist, with cleanup \
                        guidance for test runs."
    )]
    async fn sqlserver_workflow_sql_agent_jobs_prompt(
        &self,
        Parameters(args): Parameters<SqlAgentJobsArgs>,
    ) -> Vec<PromptMessage> {
        let header = render_context_header(&[
            ("job_name", args.job_name.as_deref()),
            ("database", args.database.as_deref()),
        ]);
        vec![PromptMessage::new_text(
            Role::User,
            format!("{header}\n{}", include_str!("content/sql_agent_jobs.md")),
        )]
    }

    #[prompt(
        name = "sqlserver_workflow_schema_exploration",
        description = "Discover databases/schemas/tables/views/columns/types/triggers, \
                        either via `sys.*` catalog views or `INFORMATION_SCHEMA.*`."
    )]
    async fn sqlserver_workflow_schema_exploration_prompt(&self) -> Vec<PromptMessage> {
        vec![PromptMessage::new_text(
            Role::User,
            include_str!("content/schema_exploration.md"),
        )]
    }

    #[prompt(
        name = "sqlserver_workflow_indexes_constraints",
        description = "Inspect indexes, index columns, foreign keys, and check \
                        constraints on a table; estimate compression savings."
    )]
    async fn sqlserver_workflow_indexes_constraints_prompt(
        &self,
        Parameters(args): Parameters<IndexesConstraintsArgs>,
    ) -> Vec<PromptMessage> {
        let header = render_context_header(&[
            ("database", args.database.as_deref()),
            ("schema", args.schema.as_deref()),
            ("table", args.table.as_deref()),
        ]);
        vec![PromptMessage::new_text(
            Role::User,
            format!(
                "{header}\n{}",
                include_str!("content/indexes_constraints.md")
            ),
        )]
    }

    #[prompt(
        name = "sqlserver_workflow_security_provisioning",
        description = "Guided login/user/role provisioning, including the \
                        built-in-vs-custom-role fork and the new-login-vs-existing-login fork."
    )]
    async fn sqlserver_workflow_security_provisioning_prompt(
        &self,
        Parameters(args): Parameters<SecurityProvisioningArgs>,
    ) -> Vec<PromptMessage> {
        let header = render_context_header(&[
            ("login_name", args.login_name.as_deref()),
            ("database", args.database.as_deref()),
            ("role_name", args.role_name.as_deref()),
        ]);
        vec![PromptMessage::new_text(
            Role::User,
            format!(
                "{header}\n{}",
                include_str!("content/security_provisioning.md")
            ),
        )]
    }

    #[prompt(
        name = "sqlserver_workflow_server_administration",
        description = "Server/database config, renaming objects, disk-space usage, \
                        dependency lookup, bulk per-table/per-db operations, linked servers."
    )]
    async fn sqlserver_workflow_server_administration_prompt(&self) -> Vec<PromptMessage> {
        vec![PromptMessage::new_text(
            Role::User,
            include_str!("content/server_administration.md"),
        )]
    }

    #[prompt(
        name = "sqlserver_workflow_performance_diagnostics",
        description = "Thin pointer to the right read-only signal (active requests/sessions, \
                        wait stats, blocking/locks, transactions, resource governor, I/O)."
    )]
    async fn sqlserver_workflow_performance_diagnostics_prompt(&self) -> Vec<PromptMessage> {
        vec![PromptMessage::new_text(
            Role::User,
            include_str!("content/performance_diagnostics.md"),
        )]
    }
}

// `#[prompt_router]` generates `prompt_router()` as `pub(crate)` (see the
// module doc comment above), so — unlike `src/core/mcp_server.rs`'s tool
// tests — these can't live in a separate `tests/*.rs` integration binary
// (that compiles against only the crate's *public* API). They're colocated
// here instead, matching this repo's own convention of keeping a file's
// tests alongside the code they exercise (see `mcp_server.rs`, `store.rs`).
#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::auth_manager::AuthManager;
    use crate::core::config_schema::AuthMethod;

    fn server() -> McpifyServer {
        let config: crate::core::config_schema::Config =
            serde_json::from_value(serde_json::json!({
                "url": "localhost",
                "auth_method": "sql_server"
            }))
            .unwrap();
        McpifyServer::new(
            "2025".to_string(),
            config,
            std::sync::Arc::new(tokio::sync::Mutex::new(AuthManager::new(
                AuthMethod::SqlServer,
            ))),
        )
    }

    #[test]
    fn prompt_router_registers_every_prompt_name() {
        let prompts = McpifyServer::prompt_router().list_all();
        let names: std::collections::BTreeSet<&str> =
            prompts.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names,
            std::collections::BTreeSet::from([
                "sqlserver_workflow",
                "sqlserver_workflow_sql_agent_jobs",
                "sqlserver_workflow_schema_exploration",
                "sqlserver_workflow_indexes_constraints",
                "sqlserver_workflow_security_provisioning",
                "sqlserver_workflow_server_administration",
                "sqlserver_workflow_performance_diagnostics",
            ])
        );
    }

    #[test]
    fn security_provisioning_advertises_every_argument_as_optional() {
        let prompts = McpifyServer::prompt_router().list_all();
        let security = prompts
            .iter()
            .find(|p| p.name == "sqlserver_workflow_security_provisioning")
            .expect("sqlserver_workflow_security_provisioning is registered");
        let arguments = security
            .arguments
            .as_ref()
            .expect("prompt declares arguments");
        let names: std::collections::BTreeSet<&str> =
            arguments.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(
            names,
            std::collections::BTreeSet::from(["login_name", "database", "role_name"])
        );
        for arg in arguments {
            assert_ne!(
                arg.required,
                Some(true),
                "{} must not be a required argument",
                arg.name
            );
        }
    }

    #[tokio::test]
    async fn master_prompt_links_to_the_sql_agent_jobs_sub_workflow() {
        let messages = server()
            .sqlserver_workflow_prompt(Parameters(MasterWorkflowArgs { goal: None }))
            .await;
        let text = &messages[0].content.as_text().unwrap().text;
        assert!(text.contains("sqlserver_workflow_sql_agent_jobs"));
    }

    #[tokio::test]
    async fn sql_agent_jobs_prompt_echoes_supplied_args_and_lists_missing_ones() {
        let messages = server()
            .sqlserver_workflow_sql_agent_jobs_prompt(Parameters(SqlAgentJobsArgs {
                job_name: Some("nightly_backup".to_string()),
                database: None,
            }))
            .await;
        let text = &messages[0].content.as_text().unwrap().text;
        assert!(text.contains("- job_name: nightly_backup"));
        assert!(text.contains("- database"));
        assert!(!text.contains("- database: "));
    }
}
