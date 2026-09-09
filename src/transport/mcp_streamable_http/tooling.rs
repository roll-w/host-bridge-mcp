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
use rmcp::ErrorData as McpError;
use rmcp::model::CallToolResult;
use serde_json::json;

pub(super) async fn execute_command_tool(
    server: &HostBridgeMcpServer,
    args: ExecuteCommandToolArgs,
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
        let approved = match server
            .operator_console
            .request_confirmation(prepared.execution_id(), request)
            .await
        {
            Ok(approved) => approved,
            Err(ConsoleApprovalError::TimedOut) => {
                return Ok(structured_error("command confirmation timed out"));
            }
            Err(ConsoleApprovalError::Cancelled) => {
                return Ok(structured_error(
                    "command confirmation was cancelled before completion",
                ));
            }
        };

        if !approved {
            if let Err(error) = server.execution_service.record_rejected(&prepared) {
                tracing::error!(
                    execution_id = %prepared.execution_id(),
                    error = %error,
                    "Failed to persist rejected execution history"
                );
            }
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
                    last_status_message = message;

                    if matches!(
                        final_state,
                        ExecutionState::Completed | ExecutionState::Failed
                    ) {
                        break;
                    }
                }
                ExecutionEvent::Output { .. } => {}
                ExecutionEvent::Exit {
                    code,
                    success,
                    timed_out,
                } => {
                    exit_code = Some(code);
                    exit_success = Some(success);
                    exit_timed_out = Some(timed_out);
                }
                ExecutionEvent::Error { .. } => {}
            },
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
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
