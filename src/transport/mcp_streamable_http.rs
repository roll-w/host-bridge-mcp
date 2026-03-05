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

use crate::application::execution_service::{
    ConfirmationRequest, ExecuteCommandInput, ExecutionError, ExecutionEvent, ExecutionService,
    ExecutionState, OutputKind,
};
use axum::extract::{Path, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use rmcp::handler::server::{router::tool::ToolRouter, wrapper::Parameters};
use rmcp::model::{
    CallToolResult, CreateElicitationRequestParams, ElicitationAction, ElicitationSchema,
    Implementation, LoggingLevel, LoggingMessageNotificationParam, ProtocolVersion,
    ServerCapabilities, ServerInfo, SetLevelRequestParams,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use rmcp::{schemars, tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::convert::Infallible;
use std::time::Duration;
use tokio_stream::once;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;
use uuid::Uuid;

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
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl HostBridgeMcpServer {
    pub fn new(execution_service: ExecutionService) -> Self {
        Self {
            execution_service,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(description = "Execute a single command string on host toolchains and stream output.")]
    async fn execute_command(
        &self,
        Parameters(args): Parameters<ExecuteCommandToolArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let input = ExecuteCommandInput {
            command: args.command,
            working_directory: args.working_directory,
            env: args.env,
            timeout_ms: args.timeout_ms,
        };

        let (_launch, mut receiver) =
            match self.execution_service.submit_command_stream(input.clone()).await {
                Ok(result) => result,
                Err(ExecutionError::ConfirmationRequired(request)) => {
                    let _ = notify_mcp_log(
                        &context.peer,
                        LoggingLevel::Info,
                        json!({
                        "type": "confirmation_required",
                        "preview": request,
                    }),
                    )
                        .await;

                    let approved = match request_user_confirmation(&context.peer, &request).await {
                        Ok(approved) => approved,
                        Err(message) => {
                            return Ok(CallToolResult::structured_error(json!({ "message": message })));
                        }
                    };

                    if !approved {
                        return Ok(CallToolResult::structured_error(json!({
                        "message": "command confirmation was rejected"
                    })));
                    }

                    match self.execution_service.submit_command_stream(input.clone()).await {
                        Ok(result) => result,
                        Err(error) => {
                            return Ok(CallToolResult::structured_error(json!({
                            "message": error.to_string()
                        })));
                        }
                    }
                }
                Err(error) => {
                    return Ok(CallToolResult::structured_error(json!({
                    "message": error.to_string()
                })));
                }
            };

        let mut final_state = ExecutionState::Running;
        let mut exit_code: Option<i32> = None;
        let mut exit_success: Option<bool> = None;
        let mut exit_timed_out: Option<bool> = None;
        let mut last_status_message: Option<String> = None;

        loop {
            match receiver.recv().await {
                Ok(event) => {
                    match event {
                        ExecutionEvent::Status { state, message } => {
                            final_state = state;
                            last_status_message = message.clone();
                            let _ = notify_mcp_log(
                                &context.peer,
                                LoggingLevel::Info,
                                json!({
                                    "type": "status",
                                    "state": final_state,
                                    "message": message,
                                }),
                            )
                                .await;

                            if matches!(final_state, ExecutionState::Completed | ExecutionState::Failed) {
                                break;
                            }
                        }
                        ExecutionEvent::Output { stream, text } => {
                            let level = match stream {
                                OutputKind::Stdout => LoggingLevel::Info,
                                OutputKind::Stderr => LoggingLevel::Error,
                            };
                            let _ = notify_mcp_log(
                                &context.peer,
                                level,
                                json!({
                                    "type": "output",
                                    "stream": stream,
                                    "text": text,
                                }),
                            )
                                .await;
                        }
                        ExecutionEvent::Exit {
                            code,
                            success,
                            timed_out,
                        } => {
                            exit_code = Some(code);
                            exit_success = Some(success);
                            exit_timed_out = Some(timed_out);
                            let _ = notify_mcp_log(
                                &context.peer,
                                LoggingLevel::Info,
                                json!({
                                    "type": "exit",
                                    "code": code,
                                    "success": success,
                                    "timedOut": timed_out,
                                }),
                            )
                                .await;
                        }
                        ExecutionEvent::Error { message } => {
                            let _ = notify_mcp_log(
                                &context.peer,
                                LoggingLevel::Error,
                                json!({
                                    "type": "error",
                                    "message": message,
                                }),
                            )
                                .await;
                        }
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    let _ = notify_mcp_log(
                        &context.peer,
                        LoggingLevel::Warning,
                        json!({
                            "type": "lagged",
                            "skipped": skipped,
                        }),
                    )
                        .await;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    break;
                }
            }
        }

        Ok(CallToolResult::structured(json!({
            "status": final_state,
            "exit": {
                "code": exit_code.unwrap_or(-1),
                "success": exit_success.unwrap_or(false),
                "timedOut": exit_timed_out.unwrap_or(false)
            },
            "message": last_status_message,
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
            .with_server_info(Implementation::from_build_env())
            .with_protocol_version(ProtocolVersion::LATEST)
            .with_instructions(
                "Host bridge MCP server exposing execute_command tool with real-time command output stream."
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

async fn notify_mcp_log(
    peer: &rmcp::service::Peer<RoleServer>,
    level: LoggingLevel,
    data: Value,
) -> Result<(), ()> {
    if let Err(error) = peer
        .notify_logging_message(LoggingMessageNotificationParam {
            level,
            logger: Some("host-bridge-mcp".to_string()),
            data,
        })
        .await
    {
        tracing::debug!(error = %error, "Failed to send MCP logging message");
        return Err(());
    }
    Ok(())
}

async fn request_user_confirmation(
    peer: &rmcp::service::Peer<RoleServer>,
    request: &ConfirmationRequest,
) -> Result<bool, String> {
    let message = format_confirmation_message(request);
    let requested_schema = ElicitationSchema::builder()
        .required_bool("approved")
        .build()
        .map_err(|error| format!("invalid confirmation schema: {error}"))?;
    let params = CreateElicitationRequestParams::FormElicitationParams {
        meta: None,
        message,
        requested_schema,
    };

    let result = peer
        .create_elicitation(params)
        .await
        .map_err(|error| format!("confirmation required but elicitation failed: {error}"))?;

    if result.action != ElicitationAction::Accept {
        return Ok(false);
    }

    let approved = result
        .content
        .as_ref()
        .and_then(|value| value.get("approved"))
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    Ok(approved)
}

fn format_confirmation_message(request: &ConfirmationRequest) -> String {
    let mut lines = Vec::new();
    lines.push("Command requires confirmation.".to_string());
    lines.push("".to_string());
    lines.push(format!("commandLine: {}", request.command_line));
    lines.push(format!("executable : {}", request.executable));
    lines.push(format!("args       : {:?}", request.args));
    lines.push(format!("workdir    : {}", request.working_directory));
    lines.push(format!("timeoutMs  : {}", request.timeout_ms));

    if request.env.is_empty() {
        lines.push("env        : <none>".to_string());
    } else {
        lines.push("env        :".to_string());
        let mut keys = request.env.keys().collect::<Vec<_>>();
        keys.sort();
        for key in keys {
            let value = request.env.get(key).map(String::as_str).unwrap_or("");
            lines.push(format!("  {key}={value}"));
        }
    }

    lines.push("".to_string());
    lines.push("Approve execution?".to_string());
    lines.join("\n")
}

pub fn router(execution_service: ExecutionService) -> Router {
    let stream_state = HttpState {
        execution_service: execution_service.clone(),
    };

    let mcp_execution_service = execution_service.clone();
    let mcp_config = StreamableHttpServerConfig {
        stateful_mode: true,
        json_response: false,
        ..StreamableHttpServerConfig::default()
    };
    let mcp_service: StreamableHttpService<HostBridgeMcpServer, LocalSessionManager> =
        StreamableHttpService::new(
            move || {
                Ok(HostBridgeMcpServer::new(mcp_execution_service.clone()))
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

async fn health() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

async fn stream_execution(
    Path(execution_id): Path<String>,
    State(state): State<HttpState>,
) -> Result<impl IntoResponse, axum::http::StatusCode> {
    let execution_id = Uuid::parse_str(&execution_id)
        .map_err(|_| axum::http::StatusCode::BAD_REQUEST)?;

    let subscription = state
        .execution_service
        .subscribe(execution_id)
        .await
        .map_err(|error| match error {
            ExecutionError::NotFound(_) => axum::http::StatusCode::NOT_FOUND,
            _ => axum::http::StatusCode::BAD_REQUEST,
        })?;

    let initial_event = Event::default()
        .event("status")
        .data(serialize_event(&ExecutionEvent::Status {
            state: subscription.current_state,
            message: Some("Subscribed to execution stream".to_string()),
        }));

    let initial_stream = once(Ok::<Event, Infallible>(initial_event));
    let updates = BroadcastStream::new(subscription.receiver).filter_map(|event| match event {
        Ok(event) => Some(Ok::<Event, Infallible>(to_sse_event(&event))),
        Err(_) => None,
    });

    let stream = initial_stream.chain(updates);
    Ok(Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    ))
}

fn to_sse_event(event: &ExecutionEvent) -> Event {
    Event::default()
        .event(event_name(event))
        .data(serialize_event(event))
}

fn event_name(event: &ExecutionEvent) -> &'static str {
    match event {
        ExecutionEvent::Status { .. } => "status",
        ExecutionEvent::Output { .. } => "output",
        ExecutionEvent::Exit { .. } => "exit",
        ExecutionEvent::Error { .. } => "error",
    }
}

fn serialize_event(event: &ExecutionEvent) -> String {
    serde_json::to_string(event).unwrap_or_else(|error| {
        json!({
            "type": "error",
            "message": format!("failed to serialize event: {error}")
        })
            .to_string()
    })
}
