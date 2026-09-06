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

use crate::transport::api::ApiError;
use crate::transport::auth::RequestAuthController;
use axum::extract::{Request, State};
use axum::http::header::HeaderMap;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use uuid::Uuid;

pub(crate) const SESSION_COOKIE_NAME: &str = "hb_ui_session";
const BOOTSTRAP_TTL: Duration = Duration::from_secs(60);
const SESSION_TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60);

#[derive(Clone)]
pub struct WebSessionController {
    state: Arc<Mutex<WebSessionState>>,
}

struct WebSessionState {
    bootstrap: Option<BootstrapToken>,
    sessions: HashMap<String, Instant>,
}

struct BootstrapToken {
    value: String,
    expires_at: Instant,
}

impl WebSessionController {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(WebSessionState {
                bootstrap: None,
                sessions: HashMap::new(),
            })),
        }
    }

    pub fn create_bootstrap_url(&self, bind_address: &str) -> String {
        let token = Uuid::new_v4().simple().to_string();
        let mut state = self.state.lock().expect("web session lock poisoned");
        prune_sessions(&mut state);
        state.bootstrap = Some(BootstrapToken {
            value: token.clone(),
            expires_at: Instant::now() + BOOTSTRAP_TTL,
        });
        drop(state);

        format!("{}/#bootstrap={token}", browser_endpoint(bind_address))
    }

    pub fn exchange_bootstrap(&self, token: &str) -> Option<String> {
        let mut state = self.state.lock().expect("web session lock poisoned");
        prune_sessions(&mut state);
        let is_valid = state.bootstrap.as_ref().is_some_and(|bootstrap| {
            bootstrap.expires_at > Instant::now()
                && constant_time_equals(bootstrap.value.as_bytes(), token.as_bytes())
        });
        if !is_valid {
            return None;
        }

        state.bootstrap = None;
        Some(issue_session(&mut state))
    }

    pub fn issue_session(&self) -> String {
        let mut state = self.state.lock().expect("web session lock poisoned");
        prune_sessions(&mut state);
        issue_session(&mut state)
    }

    pub fn authenticate_headers(&self, headers: &HeaderMap) -> bool {
        let Some(session) = session_cookie(headers) else {
            return false;
        };

        let mut state = self.state.lock().expect("web session lock poisoned");
        prune_sessions(&mut state);
        state
            .sessions
            .get(&session)
            .is_some_and(|expires_at| *expires_at > Instant::now())
    }

    pub fn revoke_headers(&self, headers: &HeaderMap) {
        if let Some(session) = session_cookie(headers) {
            self.state
                .lock()
                .expect("web session lock poisoned")
                .sessions
                .remove(&session);
        }
    }

    pub fn session_cookie(session: &str) -> String {
        format!(
            "{SESSION_COOKIE_NAME}={session}; Path=/; HttpOnly; SameSite=Strict; Max-Age={}",
            SESSION_TTL.as_secs()
        )
    }

    pub fn clear_session_cookie() -> String {
        format!("{SESSION_COOKIE_NAME}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0")
    }
}

impl Default for WebSessionController {
    fn default() -> Self {
        Self::new()
    }
}

pub(crate) async fn require_web_session(
    State((web_session, request_auth)): State<(WebSessionController, RequestAuthController)>,
    request: Request,
    next: Next,
) -> Response {
    if !request_auth.is_configured() || web_session.authenticate_headers(request.headers()) {
        next.run(request).await
    } else {
        ApiError::unauthorized("web session is required").into_response()
    }
}

fn issue_session(state: &mut WebSessionState) -> String {
    let session = Uuid::new_v4().simple().to_string();
    state
        .sessions
        .insert(session.clone(), Instant::now() + SESSION_TTL);
    session
}

fn prune_sessions(state: &mut WebSessionState) {
    let now = Instant::now();
    state.sessions.retain(|_, expires_at| *expires_at > now);
    if state
        .bootstrap
        .as_ref()
        .is_some_and(|bootstrap| bootstrap.expires_at <= now)
    {
        state.bootstrap = None;
    }
}

fn session_cookie(headers: &HeaderMap) -> Option<String> {
    headers
        .get(axum::http::header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| {
            value.split(';').find_map(|part| {
                let (name, value) = part.trim().split_once('=')?;
                (name == SESSION_COOKIE_NAME && !value.is_empty()).then(|| value.to_string())
            })
        })
}

fn browser_endpoint(bind_address: &str) -> String {
    let Some((host, port)) = split_bind_address(bind_address) else {
        return "http://127.0.0.1:8810".to_string();
    };
    let host = match host {
        "0.0.0.0" | "::" | "" => "127.0.0.1",
        value => value,
    };
    let formatted_host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_string()
    };
    format!("http://{formatted_host}:{port}")
}

fn split_bind_address(address: &str) -> Option<(&str, &str)> {
    if let Some(rest) = address.strip_prefix('[') {
        let end = rest.find(']')?;
        let host = &rest[..end];
        let port = rest.get(end + 1..)?.strip_prefix(':')?;
        return (!port.is_empty()).then_some((host, port));
    }

    let (host, port) = address.rsplit_once(':')?;
    (!host.is_empty() && !port.is_empty()).then_some((host, port))
}

fn constant_time_equals(expected: &[u8], actual: &[u8]) -> bool {
    let mut difference = expected.len() ^ actual.len();
    let max_len = expected.len().max(actual.len());
    for index in 0..max_len {
        let left = expected.get(index).copied().unwrap_or_default();
        let right = actual.get(index).copied().unwrap_or_default();
        difference |= usize::from(left ^ right);
    }
    difference == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderValue, Request};

    #[test]
    fn bootstrap_tokens_are_single_use() {
        let controller = WebSessionController::new();
        let url = controller.create_bootstrap_url("127.0.0.1:8787");
        let token = url
            .split_once("#bootstrap=")
            .expect("bootstrap fragment should exist")
            .1;

        let session = controller
            .exchange_bootstrap(token)
            .expect("bootstrap token should exchange");
        assert!(controller.exchange_bootstrap(token).is_none());

        let (mut parts, _) = Request::new(()).into_parts();
        parts.headers.insert(
            axum::http::header::COOKIE,
            HeaderValue::from_str(&format!("{SESSION_COOKIE_NAME}={session}"))
                .expect("cookie should be valid"),
        );
        assert!(controller.authenticate_headers(&parts.headers));
    }

    #[test]
    fn wildcard_bind_addresses_open_on_loopback() {
        assert_eq!(browser_endpoint("0.0.0.0:9000"), "http://127.0.0.1:9000");
        assert_eq!(browser_endpoint("[::]:9000"), "http://127.0.0.1:9000");
        assert_eq!(browser_endpoint("[::1]:9000"), "http://[::1]:9000");
    }
}
