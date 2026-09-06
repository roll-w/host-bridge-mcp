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

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use serde_json::Value;

pub const OK: i32 = 0;
pub const BAD_REQUEST: i32 = 40001;
pub const UNAUTHORIZED: i32 = 40101;
pub const FORBIDDEN: i32 = 40301;
pub const NOT_FOUND: i32 = 40401;
pub const METHOD_NOT_ALLOWED: i32 = 40501;
pub const CONFLICT: i32 = 40901;
pub const INTERNAL_ERROR: i32 = 50001;

#[derive(Debug, Clone, Serialize)]
pub struct ApiStatus {
    pub code: i32,
    pub message: String,
}
#[derive(Debug, Clone, Serialize)]
pub struct ApiResponse<T> {
    pub status: ApiStatus,
    pub data: T,
}

#[derive(Debug, Clone)]
pub struct ApiError {
    http_status: StatusCode,
    code: i32,
    message: String,
}

impl ApiError {
    pub fn new(http_status: StatusCode, code: i32, message: impl Into<String>) -> Self {
        Self {
            http_status,
            code,
            message: message.into(),
        }
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, BAD_REQUEST, message)
    }

    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, UNAUTHORIZED, message)
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, FORBIDDEN, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, NOT_FOUND, message)
    }

    pub fn method_not_allowed(message: impl Into<String>) -> Self {
        Self::new(StatusCode::METHOD_NOT_ALLOWED, METHOD_NOT_ALLOWED, message)
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, CONFLICT, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, INTERNAL_ERROR, message)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let response = ApiResponse {
            status: ApiStatus {
                code: self.code,
                message: self.message,
            },
            data: Value::Null,
        };
        (self.http_status, Json(response)).into_response()
    }
}

pub fn success<T>(data: T) -> Json<ApiResponse<T>> {
    Json(ApiResponse {
        status: ApiStatus {
            code: OK,
            message: "OK".to_string(),
        },
        data,
    })
}

pub fn success_value(data: Value) -> Value {
    serde_json::to_value(ApiResponse {
        status: ApiStatus {
            code: OK,
            message: "OK".to_string(),
        },
        data,
    })
        .unwrap_or_else(|_| Value::Null)
}
