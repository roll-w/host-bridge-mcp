/*
 * Copyright 2026-present RollW
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *        http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

mod auth;
mod output;
mod streaming;
mod tooling;

use self::auth::{require_request_auth, resolve_request_auth};
use self::streaming::{health, stream_execution};
use self::tooling::execute_command_tool;
use crate::application::execution_service::ExecutionService;
use crate::application::operator_console::OperatorConsole;
use crate::config::AccessConfig;
use axum::middleware;
use axum::routing::get;
use axum::Router;
use rmcp::handler::server::{router::tool::ToolRouter, wrapper::Parameters};
use rmcp::model::{
    CallToolResult, Implementation, ProtocolVersion, ServerCapabilities, ServerInfo,
    SetLevelRequestParams,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use rmcp::{schemars, tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler};
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;

pub use self::auth::TransportAuthError;

const MCP_SERVER_NAME: &str = env!("CARGO_PKG_NAME");
const MCP_SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone)]
pub struct HttpState {
    execution_service: ExecutionService,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct ExecuteCommandToolArgs {
    #[schemars(
        description = "Exactly one command line to execute. Shell chaining operators such as &&, ||, ;, and | are rejected."
    )]
    command: String,
    #[serde(default)]
    #[schemars(
        description = "Optional configured execution server name. Omit this field to use the default server, which is usually 'host'."
    )]
    server: Option<String>,
    #[serde(default)]
    #[schemars(
        description = "Optional working directory for the child process. If omitted, the server uses the current directory or a policy default after path mapping."
    )]
    working_directory: Option<String>,
    #[serde(default)]
    #[schemars(
        description = "Extra environment variables merged into the child process. Existing variables are preserved unless you override the same key."
    )]
    env: HashMap<String, String>,
    #[serde(default)]
    #[schemars(
        description = "Optional execution timeout in milliseconds. If omitted, the server default applies. Values above the server maximum are clamped."
    )]
    timeout_ms: Option<u64>,
    #[serde(default)]
    #[schemars(
        description = "Optional number of leading lines to return from the merged command output after execution completes."
    )]
    head_lines: Option<u64>,
    #[serde(default)]
    #[schemars(
        description = "Optional number of trailing lines to return from the merged command output after execution completes."
    )]
    tail_lines: Option<u64>,
    #[serde(default)]
    #[schemars(
        description = "Optional maximum number of characters to return from the merged command output after line filtering. Use 0 to disable the character cap."
    )]
    max_chars: Option<u64>,
}

#[derive(Clone)]
struct HostBridgeMcpServer {
    execution_service: ExecutionService,
    operator_console: OperatorConsole,
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl HostBridgeMcpServer {
    pub fn new(execution_service: ExecutionService, operator_console: OperatorConsole) -> Self {
        Self {
            execution_service,
            operator_console,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        description = "Execute exactly one command in the selected execution server without shell chaining. If approval is required, the call stays pending until the TUI operator approves or rejects it."
    )]
    async fn execute_command(
        &self,
        Parameters(args): Parameters<ExecuteCommandToolArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        execute_command_tool(self, args, context).await
    }

    #[tool(
        description = "Return the configured execution environments with their names and platform types, plus the default environment name."
    )]
    async fn get_execution_environment(&self) -> Result<CallToolResult, McpError> {
        Ok(CallToolResult::structured(json!({
            "defaultEnvironment": self.execution_service.default_server_name(),
            "environments": self.execution_service.available_environments(),
        })))
    }
}

#[tool_handler]
impl ServerHandler for HostBridgeMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_logging()
                .build(),
        )
            .with_server_info(server_implementation())
            .with_protocol_version(ProtocolVersion::LATEST)
            .with_instructions(
                "Host bridge MCP server exposing execute_command for host processes and get_execution_environment for platform discovery."
                    .to_string(),
            )
    }

    async fn set_level(
        &self,
        request: SetLevelRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<(), McpError> {
        tracing::debug!(level = ?request.level, "Client set MCP logging level");
        Ok(())
    }
}

fn server_implementation() -> Implementation {
    Implementation::new(MCP_SERVER_NAME, MCP_SERVER_VERSION)
}

#[cfg(test)]
mod tests {
    use super::{server_implementation, ExecuteCommandToolArgs};
    use super::HostBridgeMcpServer;
    use crate::application::execution_service::ExecutionService;
    use crate::application::operator_console::OperatorConsole;
    use crate::config::{
        AppConfig, ExecutionConfig, ExecutionServerConfig, SshAuthConfig, SshAuthType,
        TargetPlatform,
    };
    use rmcp::schemars;
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::Arc;

    #[test]
    fn deserializes_output_limit_arguments() {
        let args: ExecuteCommandToolArgs = serde_json::from_value(json!({
            "command": "cargo test",
            "server": "prod",
            "workingDirectory": "/workspace/project",
            "timeoutMs": 120000,
            "headLines": 12,
            "tailLines": 8,
            "maxChars": 0,
            "env": {
                "RUST_LOG": "debug"
            }
        }))
            .expect("arguments should deserialize");

        assert_eq!(args.command, "cargo test");
        assert_eq!(args.server.as_deref(), Some("prod"));
        assert_eq!(
            args.working_directory.as_deref(),
            Some("/workspace/project")
        );
        assert_eq!(args.timeout_ms, Some(120000));
        assert_eq!(args.head_lines, Some(12));
        assert_eq!(args.tail_lines, Some(8));
        assert_eq!(args.max_chars, Some(0));
        assert_eq!(
            args.env,
            HashMap::from([(String::from("RUST_LOG"), String::from("debug"))])
        );
    }

    #[test]
    fn schema_contains_agent_visible_field_descriptions() {
        let schema = schemars::schema_for!(ExecuteCommandToolArgs);
        let schema_json = serde_json::to_string(&schema).expect("schema should serialize");

        assert!(
            schema_json.contains("Shell chaining operators such as &&, ||, ;, and | are rejected.")
        );
        assert!(schema_json.contains("Use 0 to disable the character cap."));
        assert!(schema_json.contains("merged command output"));
    }

    #[test]
    fn server_implementation_uses_package_metadata() {
        let implementation = server_implementation();

        assert_eq!(implementation.name, env!("CARGO_PKG_NAME"));
        assert_eq!(implementation.version, env!("CARGO_PKG_VERSION"));
    }

    #[tokio::test]
    async fn get_execution_environment_reports_named_environments() {
        let server = HostBridgeMcpServer::new(
            ExecutionService::new(Arc::new(AppConfig {
                execution: ExecutionConfig {
                    default_server: "prod".to_string(),
                    servers: vec![ExecutionServerConfig::Ssh {
                        name: "prod".to_string(),
                        host: "prod.example.com".to_string(),
                        port: 22,
                        user: "deploy".to_string(),
                        target_platform: TargetPlatform::Linux,
                        path_mappings: Vec::new(),
                        auth: SshAuthConfig {
                            kind: SshAuthType::Agent,
                            r#ref: None,
                        },
                        known_hosts_file: None,
                        connection_idle_timeout_ms: 30_000,
                    }],
                    ..ExecutionConfig::default()
                },
                ..AppConfig::default()
            })),
            OperatorConsole::default(),
        );

        let result = server
            .get_execution_environment()
            .await
            .expect("environment query should succeed");
        let payload = result
            .structured_content
            .expect("structured payload should exist");

        assert_eq!(payload.get("defaultEnvironment"), Some(&json!("prod")));
        let environments = payload
            .get("environments")
            .and_then(serde_json::Value::as_array)
            .expect("environments should be an array");
        assert_eq!(environments.len(), 2);
        assert!(environments.iter().any(|environment| {
            environment.get("name") == Some(&json!("prod"))
                && environment.get("platform") == Some(&json!("linux"))
        }));
        assert!(environments.iter().any(|environment| {
            environment.get("name") == Some(&json!("host"))
                && environment
                .get("platform")
                .and_then(serde_json::Value::as_str)
                .is_some()
        }));
    }
}

pub fn router(
    execution_service: ExecutionService,
    operator_console: OperatorConsole,
    access: AccessConfig,
) -> Result<Router, TransportAuthError> {
    let stream_state = HttpState {
        execution_service: execution_service.clone(),
    };

    let mcp_execution_service = execution_service.clone();
    let mcp_operator_console = operator_console.clone();
    let mcp_config = StreamableHttpServerConfig {
        stateful_mode: true,
        json_response: false,
        ..StreamableHttpServerConfig::default()
    };
    let mcp_service: StreamableHttpService<HostBridgeMcpServer, LocalSessionManager> =
        StreamableHttpService::new(
            move || {
                Ok(HostBridgeMcpServer::new(
                    mcp_execution_service.clone(),
                    mcp_operator_console.clone(),
                ))
            },
            LocalSessionManager::default().into(),
            mcp_config,
        );

    let auth_state = resolve_request_auth(&access)?;
    let protected_routes = Router::new()
        .route("/executions/{execution_id}/stream", get(stream_execution))
        .nest_service("/mcp", mcp_service)
        .route_layer(middleware::from_fn_with_state(
            auth_state,
            require_request_auth,
        ));

    Ok(Router::new()
        .route("/health", get(health))
        .merge(protected_routes)
        .with_state(stream_state))
}
