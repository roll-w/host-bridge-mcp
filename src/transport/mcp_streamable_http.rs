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
    ExecuteCommandInput, ExecutionError, ExecutionEvent, ExecutionService, ExecutionState,
    OutputKind,
};
use crate::application::operator_console::{ConsoleApprovalError, OperatorConsole};
use axum::extract::{Path, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use rmcp::handler::server::{router::tool::ToolRouter, wrapper::Parameters};
use rmcp::model::{
    CallToolResult, Implementation, LoggingLevel, LoggingMessageNotificationParam,
    ProtocolVersion, ServerCapabilities, ServerInfo, SetLevelRequestParams,
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
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;
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
    /// Run exactly one executable plus its arguments.
    /// Shell chaining operators such as `&&`, `||`, `;`, and `|` are rejected.
    command: String,
    /// Optional host working directory.
    /// The server applies configured path mappings before execution.
    #[serde(default)]
    working_directory: Option<String>,
    /// Optional environment variables injected into the child process.
    #[serde(default)]
    env: HashMap<String, String>,
    /// Optional timeout in milliseconds.
    /// Values above the server maximum are clamped.
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
        let input = ExecuteCommandInput {
            command: args.command,
            working_directory: args.working_directory,
            env: args.env,
            timeout_ms: args.timeout_ms,
        };

        let prepared = match self.execution_service.prepare_command(input).await {
            Ok(prepared) => prepared,
            Err(error) => {
                return Ok(CallToolResult::structured_error(json!({
                    "message": error.to_string()
                })));
            }
        };

        if let Some(request) = prepared.confirmation_request().cloned() {
            let _ = notify_mcp_log(
                &context.peer,
                LoggingLevel::Info,
                json!({
                    "type": "approval_pending",
                    "preview": request,
                }),
            )
                .await;

            let approved = match self.operator_console.request_confirmation(request.clone()).await {
                Ok(approved) => approved,
                Err(ConsoleApprovalError::Unavailable) => {
                    return Ok(CallToolResult::structured_error(json!({
                        "message": "command requires confirmation but the TUI is unavailable"
                    })));
                }
                Err(ConsoleApprovalError::Cancelled) => {
                    return Ok(CallToolResult::structured_error(json!({
                        "message": "command confirmation was cancelled before completion"
                    })));
                }
            };

            let _ = notify_mcp_log(
                &context.peer,
                LoggingLevel::Info,
                json!({
                    "type": "approval_resolved",
                    "approved": approved,
                }),
            )
                .await;

            if !approved {
                return Ok(CallToolResult::structured_error(json!({
                    "message": "command confirmation was rejected"
                })));
            }
        }

        let (launch, mut receiver) = match self.execution_service.launch_prepared_command(prepared).await {
            Ok(result) => result,
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
        let mut output_accumulator = match OutputAccumulator::new() {
            Ok(accumulator) => accumulator,
            Err(error) => {
                return Ok(CallToolResult::structured_error(json!({
                    "message": format!("failed to initialize output spool files: {error}")
                })));
            }
        };

        loop {
            match receiver.recv().await {
                Ok(event) => match event {
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
                        output_accumulator.push(&stream, &text);

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
                },
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

        let (stdout, stderr) = output_accumulator.finish();

        Ok(CallToolResult::structured(json!({
            "executionId": launch.execution_id,
            "status": final_state,
            "exit": {
                "code": exit_code.unwrap_or(-1),
                "success": exit_success.unwrap_or(false),
                "timedOut": exit_timed_out.unwrap_or(false)
            },
            "message": last_status_message,
            "stdout": stdout,
            "stderr": stderr,
        })))
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

struct OutputAccumulator {
    stdout: OutputSpool,
    stderr: OutputSpool,
}

impl OutputAccumulator {
    fn new() -> io::Result<Self> {
        Ok(Self {
            stdout: OutputSpool::new("stdout")?,
            stderr: OutputSpool::new("stderr")?,
        })
    }

    fn push(&mut self, stream: &OutputKind, text: &str) {
        let spool = match stream {
            OutputKind::Stdout => &mut self.stdout,
            OutputKind::Stderr => &mut self.stderr,
        };

        spool.append(text);
    }

    fn finish(self) -> (String, String) {
        (self.stdout.read_all(), self.stderr.read_all())
    }
}

struct OutputSpool {
    path: PathBuf,
    file: File,
}

impl OutputSpool {
    fn new(prefix: &str) -> io::Result<Self> {
        let path = std::env::temp_dir().join(format!("host-bridge-mcp-{prefix}-{}.log", Uuid::new_v4()));
        let file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&path)?;

        Ok(Self { path, file })
    }

    fn append(&mut self, text: &str) {
        let _ = self.file.write_all(text.as_bytes());
    }

    fn read_all(mut self) -> String {
        let _ = self.file.flush();
        fs::read_to_string(&self.path).unwrap_or_default()
    }
}

impl Drop for OutputSpool {
    fn drop(&mut self) {
        let _ = self.file.flush();
        let _ = fs::remove_file(&self.path);
    }
}
