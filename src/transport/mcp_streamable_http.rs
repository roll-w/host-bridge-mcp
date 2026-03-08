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

mod output;
mod streaming;
mod tooling;

use self::streaming::{health, stream_execution};
use self::tooling::execute_command_tool;
use crate::application::execution_service::ExecutionService;
use crate::application::operator_console::OperatorConsole;
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
use std::collections::HashMap;

#[derive(Clone)]
pub struct HttpState {
    execution_service: ExecutionService,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct ExecuteCommandToolArgs {
    command: String,
    #[serde(default)]
    working_directory: Option<String>,
    #[serde(default)]
    env: HashMap<String, String>,
    #[serde(default)]
    timeout_ms: Option<u64>,
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
        description = "Execute exactly one host command without shell chaining. If approval is required, the call stays \
        pending until the TUI operator approves or rejects it, then returns the full stdout and stderr after the process \
        exits."
    )]
    async fn execute_command(
        &self,
        Parameters(args): Parameters<ExecuteCommandToolArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        execute_command_tool(self, args, context).await
    }
}

#[tool_handler]
impl ServerHandler for HostBridgeMcpServer {
    async fn set_level(
        &self,
        request: SetLevelRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<(), McpError> {
        tracing::debug!(level = ?request.level, "Client set MCP logging level");
        Ok(())
    }

    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_logging()
                .build(),
        )
            .with_server_info(Implementation::from_build_env())
            .with_protocol_version(ProtocolVersion::LATEST)
            .with_instructions(
                "Host bridge MCP server exposing execute_command with TUI approvals, blocking completion, and full stdout/stderr return."
                    .to_string(),
            )
    }
}

pub fn router(execution_service: ExecutionService, operator_console: OperatorConsole) -> Router {
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

    Router::new()
        .route("/health", get(health))
        .route("/executions/{execution_id}/stream", get(stream_execution))
        .nest_service("/mcp", mcp_service)
        .with_state(stream_state)
}
