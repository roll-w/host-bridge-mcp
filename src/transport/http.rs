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

mod api_handlers;
mod execution_stream;
mod session;
mod static_assets;

use self::api_handlers::{
    delete_history, exchange_session, get_config, get_history_entry, get_history_output,
    get_history_page, get_logs, get_overview, get_session_status, list_approvals, login_session,
    logout_session, resolve_approval, runtime_log_stream, save_raw_config, save_visual_config,
    write_ssh_password_file,
};
use self::execution_stream::stream_execution;
use self::session::require_web_session;
use crate::application::config_store::ConfigStore;
use crate::application::data_dir::DataDirectory;
use crate::application::execution_service::ExecutionService;
use crate::application::operator_console::OperatorConsole;
use crate::config::{AppConfig, ResolvedConfigPath};
use crate::transport::api::ApiError;
use crate::transport::auth::{RequestAuthController, require_request_auth};
use axum::Router;
use axum::extract::Request;
use axum::http::StatusCode;
use axum::http::header::ALLOW;
use axum::middleware;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use std::sync::Arc;

pub(crate) use self::session::WebSessionController;

#[derive(Clone)]
pub(crate) struct HttpState {
    pub(crate) execution_service: ExecutionService,
    pub(crate) operator_console: OperatorConsole,
    pub(crate) auth_controller: RequestAuthController,
    pub(crate) config_store: ConfigStore,
    pub(crate) config_fallback: Arc<AppConfig>,
    pub(crate) web_session: WebSessionController,
    pub(crate) data_directory: DataDirectory,
}

pub(crate) fn router(
    execution_service: ExecutionService,
    operator_console: OperatorConsole,
    auth_controller: RequestAuthController,
    config_path: ResolvedConfigPath,
    config_fallback: Arc<AppConfig>,
    web_session: WebSessionController,
    data_directory: DataDirectory,
) -> Router {
    let state = HttpState {
        execution_service: execution_service.clone(),
        operator_console,
        auth_controller: auth_controller.clone(),
        config_store: ConfigStore::new(config_path),
        config_fallback,
        web_session: web_session.clone(),
        data_directory,
    };

    let public_api = Router::<HttpState>::new()
        .route("/session/exchange", post(exchange_session))
        .route("/session/login", post(login_session))
        .route("/session/status", get(get_session_status));

    let protected_api = Router::<HttpState>::new()
        .route("/session/logout", post(logout_session))
        .route("/overview", get(get_overview))
        .route("/approvals", get(list_approvals))
        .route("/approvals/{approval_id}", post(resolve_approval))
        .route("/logs", get(get_logs))
        .route("/logs/stream", get(runtime_log_stream))
        .route("/config", get(get_config))
        .route("/config/raw", put(save_raw_config))
        .route("/config/visual", put(save_visual_config))
        .route("/config/ssh-password-file", post(write_ssh_password_file))
        .route("/history", get(get_history_page))
        .route(
            "/history/{execution_id}",
            get(get_history_entry).delete(delete_history),
        )
        .route("/history/{execution_id}/output", get(get_history_output))
        .route("/executions/{execution_id}/stream", get(stream_execution))
        .route_layer(middleware::from_fn_with_state(
            (web_session, auth_controller.clone()),
            require_web_session,
        ));

    let legacy_protected_routes = Router::<HttpState>::new()
        .route("/executions/{execution_id}/stream", get(stream_execution))
        .route_layer(middleware::from_fn_with_state(
            auth_controller,
            require_request_auth,
        ));

    Router::<HttpState>::new()
        .route("/", get(static_assets::index))
        .route("/icon.svg", get(static_assets::icon))
        .route("/assets/app.js", get(static_assets::app_js))
        .route("/assets/app.css", get(static_assets::app_css))
        .route("/health", get(api_handlers::health))
        .nest("/api/v1", public_api.merge(protected_api))
        .merge(legacy_protected_routes)
        .merge(crate::transport::mcp_streamable_http::router(
            execution_service,
            state.operator_console.clone(),
            state.auth_controller.clone(),
        ))
        .fallback(api_handlers::not_found)
        .with_state(state)
        .layer(middleware::from_fn(normalize_api_errors))
}

async fn normalize_api_errors(request: Request, next: middleware::Next) -> Response {
    let is_api_path =
        request.uri().path() == "/api/v1" || request.uri().path().starts_with("/api/v1/");
    let response = next.run(request).await;

    if is_api_path && response.status() == StatusCode::METHOD_NOT_ALLOWED {
        let allow = response.headers().get(ALLOW).cloned();
        let mut normalized = ApiError::method_not_allowed("method is not allowed").into_response();
        if let Some(allow) = allow {
            normalized.headers_mut().insert(ALLOW, allow);
        }
        normalized
    } else {
        response
    }
}
