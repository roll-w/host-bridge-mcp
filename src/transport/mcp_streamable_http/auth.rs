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

use crate::config::AccessConfig;
use axum::extract::{Request, State};
use axum::http::header::AUTHORIZATION;
use axum::http::header::WWW_AUTHENTICATE;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use std::sync::{Arc, RwLock};

const AUTH_REALM: &str = "host-bridge-mcp";
const AUTH_SCHEME: &str = "Bearer";

#[derive(Clone)]
pub(crate) struct RequestAuthState {
    auth: Option<RequestAuth>,
}

#[derive(Clone)]
pub struct RequestAuthController {
    state: Arc<RwLock<RequestAuthState>>,
}

#[derive(Clone)]
struct RequestAuth {
    api_key: Arc<[u8]>,
}

#[derive(Debug, thiserror::Error)]
pub enum TransportAuthError {
    #[error("server.access.api-key-env '{0}' is set but the environment variable is missing")]
    MissingApiKeyEnv(String),
    #[error("server.access.api-key-env '{0}' resolved to an empty value")]
    EmptyApiKeyEnv(String),
}

impl RequestAuthController {
    pub fn new(access: &AccessConfig) -> Result<Self, TransportAuthError> {
        Ok(Self {
            state: Arc::new(RwLock::new(Self::prepare(access)?)),
        })
    }

    pub(crate) fn prepare(access: &AccessConfig) -> Result<RequestAuthState, TransportAuthError> {
        resolve_request_auth(access)
    }

    pub(crate) fn apply(&self, state: RequestAuthState) {
        *self.state.write().expect("request auth lock poisoned") = state;
    }

    fn snapshot(&self) -> RequestAuthState {
        self.state
            .read()
            .expect("request auth lock poisoned")
            .clone()
    }
}

pub(super) fn resolve_request_auth(
    access: &AccessConfig,
) -> Result<RequestAuthState, TransportAuthError> {
    let Some(env_name) = access.api_key_env.as_deref() else {
        return Ok(RequestAuthState { auth: None });
    };

    let api_key = std::env::var(env_name)
        .map_err(|_| TransportAuthError::MissingApiKeyEnv(env_name.to_string()))?;

    if api_key.trim().is_empty() {
        return Err(TransportAuthError::EmptyApiKeyEnv(env_name.to_string()));
    }

    Ok(RequestAuthState {
        auth: Some(RequestAuth {
            api_key: Arc::<[u8]>::from(api_key.into_bytes()),
        }),
    })
}

pub(super) async fn require_request_auth(
    State(controller): State<RequestAuthController>,
    request: Request,
    next: Next,
) -> Response {
    let state = controller.snapshot();
    let Some(auth) = state.auth.as_ref() else {
        return next.run(request).await;
    };

    let outcome = authenticate_request(request.headers(), auth);
    if outcome.is_authorized {
        return next.run(request).await;
    }

    tracing::warn!(reason = %outcome.reason, header = %AUTHORIZATION, "Rejected unauthorized request");
    unauthorized_response()
}

struct AuthOutcome {
    is_authorized: bool,
    reason: &'static str,
}

fn authenticate_request(headers: &HeaderMap, auth: &RequestAuth) -> AuthOutcome {
    let Some(value) = headers.get(AUTHORIZATION) else {
        return AuthOutcome {
            is_authorized: false,
            reason: "missing_header",
        };
    };

    let Ok(value) = value.to_str() else {
        return AuthOutcome {
            is_authorized: false,
            reason: "invalid_header_encoding",
        };
    };

    let Some((scheme, token)) = split_scheme_and_token(value) else {
        return AuthOutcome {
            is_authorized: false,
            reason: "invalid_header_format",
        };
    };

    if !scheme.eq_ignore_ascii_case(AUTH_SCHEME) {
        return AuthOutcome {
            is_authorized: false,
            reason: "invalid_scheme",
        };
    }

    AuthOutcome {
        is_authorized: constant_time_equals(&auth.api_key, token.as_bytes()),
        reason: "invalid_api_key",
    }
}

fn split_scheme_and_token(value: &str) -> Option<(&str, &str)> {
    let (scheme, token) = value.split_once(' ')?;
    let token = token.trim();
    if scheme.is_empty() || token.is_empty() {
        return None;
    }

    Some((scheme, token))
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

fn unauthorized_response() -> Response {
    let mut response = StatusCode::UNAUTHORIZED.into_response();
    let header_value = format!(r#"{AUTH_SCHEME} realm="{AUTH_REALM}""#);

    if let Ok(value) = HeaderValue::from_str(&header_value) {
        response.headers_mut().insert(WWW_AUTHENTICATE, value);
    } else {
        response.headers_mut().insert(
            WWW_AUTHENTICATE,
            HeaderValue::from_static("Bearer realm=\"host-bridge-mcp\""),
        );
    }

    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::header::AUTHORIZATION;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn split_scheme_and_token_requires_both_parts() {
        assert_eq!(
            split_scheme_and_token("Bearer token"),
            Some(("Bearer", "token"))
        );
        assert_eq!(split_scheme_and_token("Bearer"), None);
        assert_eq!(split_scheme_and_token("Bearer   "), None);
    }

    #[test]
    fn constant_time_equals_checks_content_and_length() {
        assert!(constant_time_equals(b"secret", b"secret"));
        assert!(!constant_time_equals(b"secret", b"secret2"));
        assert!(!constant_time_equals(b"secret", b"SECRET"));
    }

    #[test]
    fn authenticate_request_accepts_matching_authorization_header() {
        let headers =
            HeaderMap::from_iter([(AUTHORIZATION, HeaderValue::from_static("Bearer test-key"))]);
        let auth = RequestAuth {
            api_key: Arc::<[u8]>::from(&b"test-key"[..]),
        };

        let outcome = authenticate_request(&headers, &auth);
        assert!(outcome.is_authorized);
    }

    #[test]
    fn resolve_request_auth_reads_secret_from_environment() {
        let _guard = env_lock().lock().expect("env lock should not be poisoned");
        let env_name = "HOST_BRIDGE_TEST_API_KEY";
        // SAFETY: Tests serialize environment mutation with a global mutex.
        unsafe {
            std::env::set_var(env_name, "test-key");
        }

        let result = resolve_request_auth(&AccessConfig {
            api_key_env: Some(env_name.to_string()),
        });

        // SAFETY: Tests serialize environment mutation with a global mutex.
        unsafe {
            std::env::remove_var(env_name);
        }

        assert!(result.expect("auth should resolve").auth.is_some());
    }
}
