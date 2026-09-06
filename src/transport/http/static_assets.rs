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

use axum::body::Body;
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};

const INDEX_HTML: &str = include_str!("../../../web/dist/index.html");
const ICON_SVG: &str = include_str!("../../../web/dist/icon.svg");
const APP_JS: &[u8] = include_bytes!("../../../web/dist/assets/app.js");
const APP_CSS: &[u8] = include_bytes!("../../../web/dist/assets/app.css");

pub(crate) async fn index() -> impl IntoResponse {
    ([(CONTENT_TYPE, "text/html; charset=utf-8")], INDEX_HTML)
}

pub(crate) async fn icon() -> impl IntoResponse {
    ([(CONTENT_TYPE, "image/svg+xml")], ICON_SVG)
}

pub(crate) async fn app_js() -> Response {
    asset(APP_JS, "application/javascript; charset=utf-8")
}

pub(crate) async fn app_css() -> Response {
    asset(APP_CSS, "text/css; charset=utf-8")
}

fn asset(content: &'static [u8], content_type: &'static str) -> Response {
    let mut response = Response::new(Body::from(content));
    *response.status_mut() = StatusCode::OK;
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static(content_type));
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    response
}
