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

use crate::application::execution_service::{ExecuteCommandInput, ExecutionEvent, ExecutionState};
use crate::application::operator_console::ConsoleApprovalError;
use crate::transport::mcp_streamable_http::output::OutputRenderOptions;
use crate::transport::mcp_streamable_http::{ExecuteCommandToolArgs, HostBridgeMcpServer};
use rmcp::model::{CallToolResult, LoggingLevel, LoggingMessageNotificationParam};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::ErrorData as McpError;
use serde_json::{json, Value};

pub(super) async fn execute_command_tool(
    server: &HostBridgeMcpServer,
    args: ExecuteCommandToolArgs,
    context: RequestContext<RoleServer>,
) -> Result<CallToolResult, McpError> {
    let output_options = OutputRenderOptions::new(args.head_lines, args.tail_lines, args.max_chars);
    let input = ExecuteCommandInput {
        command: args.command,
        server: args.server,
        working_directory: args.working_directory,
        env: args.env,
        timeout_ms: args.timeout_ms,
    };

    let prepared = match server.execution_service.prepare_command(input).await {
        Ok(prepared) => prepared,
        Err(error) => return Ok(structured_error(error.to_string())),
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

        let approved = match server
            .operator_console
            .request_confirmation(request.clone())
            .await
        {
            Ok(approved) => approved,
            Err(ConsoleApprovalError::Unavailable) => {
                return Ok(structured_error(
                    "command requires confirmation but the TUI is unavailable",
                ));
            }
            Err(ConsoleApprovalError::Cancelled) => {
                return Ok(structured_error(
                    "command confirmation was cancelled before completion",
                ));
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
            return Ok(structured_error("command confirmation was rejected"));
        }
    }

    let (launch, mut receiver) = match server
        .execution_service
        .launch_prepared_command(prepared)
        .await
    {
        Ok(result) => result,
        Err(error) => return Ok(structured_error(error.to_string())),
    };

    let mut final_state = ExecutionState::Running;
    let mut exit_code: Option<i32> = None;
    let mut exit_success: Option<bool> = None;
    let mut exit_timed_out: Option<bool> = None;
    let mut last_status_message: Option<String> = None;

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

                    if matches!(
                        final_state,
                        ExecutionState::Completed | ExecutionState::Failed
                    ) {
                        break;
                    }
                }
                ExecutionEvent::Output { text } => {
                    let _ = notify_mcp_log(
                        &context.peer,
                        LoggingLevel::Info,
                        json!({
                            "type": "output",
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

    let output = match server
        .execution_service
        .read_output(launch.execution_id)
        .await
    {
        Ok(output) => output_options.apply(output),
        Err(error) => return Ok(structured_error(error.to_string())),
    };

    Ok(CallToolResult::structured(json!({
        "executionId": launch.execution_id,
        "status": final_state,
        "exit": {
            "code": exit_code.unwrap_or(-1),
            "success": exit_success.unwrap_or(false),
            "timedOut": exit_timed_out.unwrap_or(false)
        },
        "message": last_status_message,
        "output": output,
    })))
}

fn structured_error(message: impl Into<String>) -> CallToolResult {
    CallToolResult::structured_error(json!({
        "message": message.into()
    }))
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
