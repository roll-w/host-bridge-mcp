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

use super::HttpState;
use super::session::WebSessionController;
use crate::application::config_store::{ConfigStoreError, VisualConfigPatch};
use crate::application::execution_service::ExecutionError;
use crate::application::operator_console::{ApprovalDecision, ConsoleLogEntry};
use crate::transport::api::{self, ApiError};
use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, RawQuery, State};
use axum::http::header::{AUTHORIZATION, SET_COOKIE};
use axum::http::{HeaderMap, HeaderValue};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::convert::Infallible;
use std::time::Duration;
use tokio_stream::StreamExt;
use tokio_stream::once;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct BootstrapRequest {
    bootstrap_token: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct LoginRequest {
    api_key: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct ApprovalDecisionRequest {
    decision: ApprovalDecisionValue,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum ApprovalDecisionValue {
    ApproveOnce,
    ApproveForSession,
    Reject,
}

#[derive(Debug, Deserialize)]
pub(super) struct RawConfigRequest {
    raw: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SshPasswordFileRequest {
    path: Option<String>,
    server_name: String,
    password: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeLogPage {
    entries: Vec<ConsoleLogEntry>,
    offset: usize,
    limit: usize,
}

pub(crate) async fn health() -> impl IntoResponse {
    api::success(json!({ "service": "host-bridge-mcp", "status": "ok" }))
}

pub(crate) async fn not_found() -> ApiError {
    ApiError::not_found("route was not found")
}

pub(crate) async fn exchange_session(
    State(state): State<HttpState>,
    payload: Result<Json<BootstrapRequest>, JsonRejection>,
) -> Result<Response, ApiError> {
    let request = parse_json(payload)?;
    let Some(session) = state
        .web_session
        .exchange_bootstrap(&request.bootstrap_token)
    else {
        return Err(ApiError::unauthorized(
            "the bootstrap token is invalid or expired",
        ));
    };

    Ok(with_session_cookie(
        api::success(json!({
            "authenticated": true,
            "apiKeyConfigured": state.auth_controller.is_configured(),
        }))
            .into_response(),
        WebSessionController::session_cookie(&session),
    ))
}

pub(crate) async fn login_session(
    State(state): State<HttpState>,
    payload: Result<Json<LoginRequest>, JsonRejection>,
) -> Result<Response, ApiError> {
    let request = parse_json(payload)?;
    if !state.auth_controller.is_configured() {
        return Err(ApiError::unauthorized(
            "API key authentication is not configured",
        ));
    }

    let authorization = HeaderValue::from_str(&format!("Bearer {}", request.api_key))
        .map_err(|_| ApiError::bad_request("api_key contains invalid header characters"))?;
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, authorization);
    if !state.auth_controller.authenticate_headers(&headers) {
        return Err(ApiError::unauthorized("invalid API key"));
    }

    let session = state.web_session.issue_session();
    Ok(with_session_cookie(
        api::success(json!({
            "authenticated": true,
            "apiKeyConfigured": true,
        }))
            .into_response(),
        WebSessionController::session_cookie(&session),
    ))
}

pub(crate) async fn get_session_status(
    State(state): State<HttpState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let authenticated =
        !state.auth_controller.is_configured() || state.web_session.authenticate_headers(&headers);
    api::success(json!({
        "authenticated": authenticated,
        "apiKeyConfigured": state.auth_controller.is_configured(),
    }))
}

pub(crate) async fn logout_session(State(state): State<HttpState>, headers: HeaderMap) -> Response {
    state.web_session.revoke_headers(&headers);
    with_session_cookie(
        api::success(json!({ "authenticated": false })).into_response(),
        WebSessionController::clear_session_cookie().to_string(),
    )
}

pub(crate) async fn get_overview(State(state): State<HttpState>) -> impl IntoResponse {
    let console = state.operator_console.snapshot();
    api::success(json!({
        "defaultEnvironment": state.execution_service.default_server_name(),
        "environments": state.execution_service.available_environments(),
        "console": console,
        "apiKeyConfigured": state.auth_controller.is_configured(),
    }))
}

pub(crate) async fn list_approvals(State(state): State<HttpState>) -> impl IntoResponse {
    let snapshot = state.operator_console.snapshot();
    api::success(json!({
        "interactive": snapshot.interactive,
        "items": snapshot.pending_approvals,
    }))
}

pub(crate) async fn resolve_approval(
    State(state): State<HttpState>,
    Path(approval_id): Path<String>,
    payload: Result<Json<ApprovalDecisionRequest>, JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    let approval_id = Uuid::parse_str(&approval_id)
        .map_err(|_| ApiError::bad_request("approval_id must be a valid UUID"))?;
    let request = parse_json(payload)?;
    let decision = match request.decision {
        ApprovalDecisionValue::ApproveOnce => ApprovalDecision::ApproveOnce,
        ApprovalDecisionValue::ApproveForSession => ApprovalDecision::ApproveForSession,
        ApprovalDecisionValue::Reject => ApprovalDecision::Reject,
    };
    if !state
        .operator_console
        .resolve_confirmation(approval_id, decision)
    {
        return Err(ApiError::not_found("approval request was not found"));
    }

    Ok(api::success(json!({ "resolved": true })))
}

pub(crate) async fn get_logs(
    State(state): State<HttpState>,
    RawQuery(raw_query): RawQuery,
) -> Result<impl IntoResponse, ApiError> {
    let (offset, limit) = parse_pagination(raw_query.as_deref())?;
    let entries = state.operator_console.read_runtime_logs(offset, limit);
    Ok(api::success(RuntimeLogPage {
        entries,
        offset,
        limit,
    }))
}

pub(crate) async fn runtime_log_stream(
    State(state): State<HttpState>,
) -> Result<impl IntoResponse, ApiError> {
    let entries = state.operator_console.read_runtime_logs(0, 100);
    let receiver = state.operator_console.subscribe_runtime_logs();
    let initial = once(Ok::<Event, Infallible>(
        Event::default().event("snapshot").data(
            api::success_value(json!({
                "entries": entries,
            }))
                .to_string(),
        ),
    ));
    let updates =
        tokio_stream::wrappers::BroadcastStream::new(receiver).filter_map(|result| match result {
            Ok(entry) => Some(Ok::<Event, Infallible>(
                Event::default()
                    .event("log")
                    .data(api::success_value(json!(entry)).to_string()),
            )),
            Err(BroadcastStreamRecvError::Lagged(skipped)) => Some(Ok::<Event, Infallible>(
                Event::default()
                    .event("lagged")
                    .data(api::success_value(json!({ "skipped": skipped })).to_string()),
            )),
        });

    Ok(Sse::new(initial.chain(updates)).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    ))
}

pub(crate) async fn get_config(
    State(state): State<HttpState>,
) -> Result<impl IntoResponse, ApiError> {
    let snapshot = state
        .config_store
        .snapshot(&state.config_fallback)
        .map_err(config_error_to_api)?;
    Ok(api::success(json!({
        "path": snapshot.path,
        "raw": snapshot.raw,
        "config": snapshot.config,
    })))
}

pub(crate) async fn save_raw_config(
    State(state): State<HttpState>,
    payload: Result<Json<RawConfigRequest>, JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    let request = parse_json(payload)?;
    let snapshot = state
        .config_store
        .save_raw(request.raw)
        .map_err(config_error_to_api)?;
    Ok(api::success(json!({
        "path": snapshot.path,
        "raw": snapshot.raw,
        "config": snapshot.config,
    })))
}

pub(crate) async fn write_ssh_password_file(
    State(state): State<HttpState>,
    payload: Result<Json<SshPasswordFileRequest>, JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    let request = parse_json(payload)?;
    if request.server_name.trim().is_empty() {
        return Err(ApiError::bad_request("server_name cannot be empty"));
    }
    if request.password.is_empty() {
        return Err(ApiError::bad_request("password cannot be empty"));
    }
    let path = match request.path.as_deref().map(str::trim) {
        Some(path) if !path.is_empty() => std::path::PathBuf::from(path),
        _ => state
            .data_directory
            .ssh_password_file_path(&request.server_name)
            .map_err(|_| ApiError::internal("failed to prepare password file path"))?,
    };

    state
        .config_store
        .write_password_file(&path.to_string_lossy(), &request.password)
        .map_err(config_error_to_api)?;
    Ok(api::success(json!({
        "path": path.to_string_lossy(),
        "written": true,
    })))
}

pub(crate) async fn save_visual_config(
    State(state): State<HttpState>,
    payload: Result<Json<VisualConfigPatch>, JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    let patch = parse_json(payload)?;
    let snapshot = state
        .config_store
        .save_visual(patch, &state.config_fallback)
        .map_err(config_error_to_api)?;
    Ok(api::success(json!({
        "path": snapshot.path,
        "raw": snapshot.raw,
        "config": snapshot.config,
    })))
}

pub(crate) async fn get_history_page(
    State(state): State<HttpState>,
    RawQuery(raw_query): RawQuery,
) -> Result<impl IntoResponse, ApiError> {
    let (offset, limit) = parse_pagination(raw_query.as_deref())?;
    let page = state
        .execution_service
        .list_history(offset, limit)
        .map_err(execution_error_to_api)?;
    Ok(api::success(page))
}

pub(crate) async fn get_history_entry(
    State(state): State<HttpState>,
    Path(execution_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let execution_id = parse_execution_id(&execution_id)?;
    let entry = state
        .execution_service
        .get_history(execution_id)
        .map_err(execution_error_to_api)?
        .ok_or_else(|| ApiError::not_found("execution history was not found"))?;
    Ok(api::success(entry))
}

pub(crate) async fn get_history_output(
    State(state): State<HttpState>,
    Path(execution_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let execution_id = parse_execution_id(&execution_id)?;
    let output = state
        .execution_service
        .read_output(execution_id)
        .await
        .map_err(execution_error_to_api)?;
    Ok(api::success(json!({
        "executionId": execution_id,
        "output": output,
    })))
}

pub(crate) async fn delete_history(
    State(state): State<HttpState>,
    Path(execution_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let execution_id = parse_execution_id(&execution_id)?;
    if !state
        .execution_service
        .delete_history(execution_id)
        .map_err(execution_error_to_api)?
    {
        return Err(ApiError::not_found("execution history was not found"));
    }
    Ok(api::success(json!({ "deleted": true })))
}

pub(super) fn parse_execution_id(value: &str) -> Result<Uuid, ApiError> {
    Uuid::parse_str(value).map_err(|_| ApiError::bad_request("execution_id must be a valid UUID"))
}

pub(super) fn execution_error_to_api(error: ExecutionError) -> ApiError {
    match error {
        ExecutionError::NotFound(_) => ApiError::not_found("execution was not found"),
        ExecutionError::HistoryStore(message) => ApiError::internal(message),
        ExecutionError::OutputStore(message) => ApiError::internal(message),
        ExecutionError::Denied => ApiError::forbidden("command execution is denied by policy"),
        ExecutionError::ConfigurationChanged => {
            ApiError::conflict("execution configuration changed; retry the request")
        }
        other => ApiError::bad_request(other.to_string()),
    }
}

fn config_error_to_api(error: ConfigStoreError) -> ApiError {
    match error {
        ConfigStoreError::Config(crate::config::ConfigError::Parse { .. })
        | ConfigStoreError::Config(crate::config::ConfigError::Validation(_)) => {
            ApiError::bad_request(error.to_string())
        }
        ConfigStoreError::Config(crate::config::ConfigError::Read { .. })
        | ConfigStoreError::Io(_)
        | ConfigStoreError::Serialize(_) => ApiError::internal(error.to_string()),
    }
}

fn parse_json<T>(payload: Result<Json<T>, JsonRejection>) -> Result<T, ApiError> {
    payload
        .map(|Json(value)| value)
        .map_err(|_| ApiError::bad_request("request body must be valid JSON"))
}

fn parse_pagination(raw_query: Option<&str>) -> Result<(usize, usize), ApiError> {
    let mut offset = 0;
    let mut limit = 100;
    for pair in raw_query.unwrap_or_default().split('&') {
        if pair.is_empty() {
            continue;
        }
        let Some((key, value)) = pair.split_once('=') else {
            return Err(ApiError::bad_request(
                "query parameters must use key=value format",
            ));
        };
        match key {
            "offset" => {
                offset = value
                    .parse::<usize>()
                    .map_err(|_| ApiError::bad_request("offset must be a non-negative integer"))?;
            }
            "limit" => {
                limit = value
                    .parse::<usize>()
                    .map_err(|_| ApiError::bad_request("limit must be a non-negative integer"))?;
                if limit > 1_000 {
                    return Err(ApiError::bad_request("limit must not exceed 1000"));
                }
            }
            _ => {}
        }
    }
    Ok((offset, limit))
}

fn with_session_cookie(mut response: Response, cookie: String) -> Response {
    if let Ok(value) = HeaderValue::from_str(&cookie) {
        response.headers_mut().insert(SET_COOKIE, value);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::{ApprovalDecisionRequest, ApprovalDecisionValue, LoginRequest};

    #[test]
    fn login_request_accepts_frontend_payload() {
        let request: LoginRequest = serde_json::from_str(r#"{"apiKey":"test-key"}"#)
            .expect("frontend login payload should deserialize");

        assert_eq!(request.api_key, "test-key");
    }

    #[test]
    fn approval_decision_request_accepts_frontend_payload() {
        let request: ApprovalDecisionRequest =
            serde_json::from_str(r#"{"decision":"approve-once"}"#)
                .expect("frontend approval payload should deserialize");

        assert!(matches!(
            request.decision,
            ApprovalDecisionValue::ApproveOnce
        ));
    }
}
