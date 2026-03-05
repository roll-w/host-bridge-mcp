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

mod application;
mod cli;
mod config;
mod domain;
mod transport;

use application::execution_service::ExecutionService;
use cli::parse_args;
use config::AppConfig;
use std::sync::Arc;
use tracing_subscriber::EnvFilter;
use transport::mcp_streamable_http::router;

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    init_logging();

    let cli_options = match parse_args(std::env::args()) {
        Ok(options) => options,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    };

    let config_path = cli_options.config_path;

    let load_result = match config_path.as_deref() {
        Some(path) => AppConfig::load_with_path(Some(path)),
        None => AppConfig::load(),
    };

    let config = match load_result {
        Ok(config) => Arc::new(config),
        Err(error) => {
            tracing::error!(error = %error, "Failed to load config");
            std::process::exit(1);
        }
    };

    let execution_service = ExecutionService::new(config.clone());
    let app = router(execution_service);
    let listener = match tokio::net::TcpListener::bind(&config.server.bind_address).await {
        Ok(listener) => listener,
        Err(error) => {
            tracing::error!(
                bind_address = %config.server.bind_address,
                error = %error,
                "Failed to bind server"
            );
            std::process::exit(1);
        }
    };

    tracing::info!(bind_address = %config.server.bind_address, "host-bridge-mcp listening");
    if let Err(error) = axum::serve(listener, app).await {
        tracing::error!(error = %error, "Server stopped with error");
        std::process::exit(1);
    }
}

fn init_logging() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();
}
