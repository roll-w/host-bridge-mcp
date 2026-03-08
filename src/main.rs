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
use application::operator_console::{ConsoleLogLevel, OperatorConsole};
use application::shutdown_controller::ShutdownController;
use cli::{help_text, parse_args, version_text};
use config::AppConfig;
use domain::platform::signal::wait_for_termination_signal;
use std::io::{self, Write};
use std::process::ExitCode;
use std::sync::Arc;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::EnvFilter;
use transport::mcp_streamable_http::router;
use transport::tui;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> ExitCode {
    let cli_options = match parse_args(std::env::args()) {
        Ok(options) => options,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(2);
        }
    };

    if cli_options.show_help {
        let program_name = std::env::args()
            .next()
            .unwrap_or_else(|| "host-bridge-mcp".to_string());
        println!("{}", help_text(&program_name));
        return ExitCode::SUCCESS;
    }

    if cli_options.show_version {
        println!("{}", version_text());
        return ExitCode::SUCCESS;
    }

    let config_path = cli_options.config_path;
    let load_result = match config_path.as_deref() {
        Some(path) => AppConfig::load_with_path(Some(path)),
        None => AppConfig::load(),
    };

    let config = match load_result {
        Ok(config) => Arc::new(config),
        Err(error) => {
            eprintln!("Failed to load config: {error}");
            return ExitCode::FAILURE;
        }
    };

    let operator_console = match OperatorConsole::new(config.logging.clone()) {
        Ok(console) => console,
        Err(error) => {
            eprintln!("Failed to initialize log storage: {error}");
            return ExitCode::FAILURE;
        }
    };

    let execution_service = ExecutionService::new(config.clone(), operator_console.clone());
    let app = router(execution_service, operator_console.clone());
    let listener = match bind_server_listener(&config.server.bind_address).await {
        Ok(listener) => listener,
        Err(error) => {
            let message = format_bind_error(&config.server.bind_address, &error);
            operator_console.push_log(ConsoleLogLevel::Error, message.clone());
            eprintln!("{message}");
            return ExitCode::FAILURE;
        }
    };

    let shutdown_controller = ShutdownController::default();
    let tui_active = tui::start(operator_console.clone(), shutdown_controller.clone());
    init_logging(operator_console.clone(), !tui_active);
    spawn_system_signal_handler(shutdown_controller.clone(), operator_console.clone());

    if tui_active {
        operator_console.push_log(ConsoleLogLevel::Info, "Interactive TUI ready.");
    } else {
        operator_console.push_log(
            ConsoleLogLevel::Warn,
            "Interactive TUI unavailable; confirmation-required commands will be rejected.",
        );
    }

    tracing::info!(bind_address = %config.server.bind_address, "host-bridge-mcp listening");
    let shutdown_waiter = shutdown_controller.clone();
    let server = axum::serve(listener, app).with_graceful_shutdown(async move {
        shutdown_waiter.wait_for_shutdown().await;
    });
    if let Err(error) = server.await {
        tracing::error!(error = %error, "Server stopped with error");
        return ExitCode::FAILURE;
    }

    tracing::info!("Server shutdown completed");
    ExitCode::SUCCESS
}

async fn bind_server_listener(bind_address: &str) -> io::Result<tokio::net::TcpListener> {
    tokio::net::TcpListener::bind(bind_address).await
}

fn format_bind_error(bind_address: &str, error: &io::Error) -> String {
    if error.kind() == io::ErrorKind::AddrInUse {
        return format!(
            "Failed to bind {bind_address}: the port is already in use. Stop the other process or change `server.bind_address` in the config."
        );
    }

    format!("Failed to bind {bind_address}: {error}")
}

fn init_logging(operator_console: OperatorConsole, mirror_to_stderr: bool) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_writer(ConsoleTracingWriterFactory {
            operator_console,
            mirror_to_stderr,
        })
        .init();
}

#[derive(Clone)]
struct ConsoleTracingWriterFactory {
    operator_console: OperatorConsole,
    mirror_to_stderr: bool,
}

struct ConsoleTracingWriter {
    operator_console: OperatorConsole,
    stderr: Option<io::Stderr>,
    buffer: Vec<u8>,
}

impl<'a> MakeWriter<'a> for ConsoleTracingWriterFactory {
    type Writer = ConsoleTracingWriter;

    fn make_writer(&'a self) -> Self::Writer {
        ConsoleTracingWriter {
            operator_console: self.operator_console.clone(),
            stderr: self.mirror_to_stderr.then(io::stderr),
            buffer: Vec::with_capacity(256),
        }
    }
}

impl Write for ConsoleTracingWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if let Some(stderr) = self.stderr.as_mut() {
            stderr.write_all(buf)?;
        }

        self.buffer.extend_from_slice(buf);
        self.flush_complete_lines();
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if let Some(stderr) = self.stderr.as_mut() {
            stderr.flush()?;
        }

        self.flush_complete_lines();
        Ok(())
    }
}

impl ConsoleTracingWriter {
    fn flush_complete_lines(&mut self) {
        while let Some(position) = self.buffer.iter().position(|byte| *byte == b'\n') {
            let line = self.buffer.drain(..=position).collect::<Vec<_>>();
            let text = String::from_utf8_lossy(&line);
            let trimmed = text.trim_end_matches(['\n', '\r']);
            self.operator_console
                .push_log(ConsoleLogLevel::Info, trimmed.to_string());
        }
    }
}

impl Drop for ConsoleTracingWriter {
    fn drop(&mut self) {
        if !self.buffer.is_empty() {
            let text = String::from_utf8_lossy(&self.buffer);
            self.operator_console
                .push_log(ConsoleLogLevel::Info, text.to_string());
            self.buffer.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_bind_error_highlights_addr_in_use() {
        let error = io::Error::new(io::ErrorKind::AddrInUse, "address in use");
        let message = format_bind_error("127.0.0.1:8787", &error);

        assert!(message.contains("127.0.0.1:8787"));
        assert!(message.contains("already in use"));
        assert!(message.contains("server.bind_address"));
    }

    #[tokio::test]
    async fn bind_server_listener_returns_addr_in_use() {
        let occupied = std::net::TcpListener::bind("127.0.0.1:0")
            .expect("test listener should bind to an ephemeral port");
        let bind_address = occupied
            .local_addr()
            .expect("test listener should expose its bound address")
            .to_string();

        let error = bind_server_listener(&bind_address)
            .await
            .expect_err("second bind should fail while the first listener is active");

        assert_eq!(error.kind(), io::ErrorKind::AddrInUse);
        assert!(format_bind_error(&bind_address, &error).contains("already in use"));
    }
}

fn spawn_system_signal_handler(
    shutdown_controller: ShutdownController,
    operator_console: OperatorConsole,
) {
    tokio::spawn(async move {
        match wait_for_termination_signal().await {
            Ok(signal_name) => {
                operator_console.push_log(
                    ConsoleLogLevel::Warn,
                    format!("System signal received: {signal_name}. Shutting down server."),
                );
                let _ = shutdown_controller.request_shutdown();
            }
            Err(error) => {
                operator_console.push_log(
                    ConsoleLogLevel::Error,
                    format!("Failed to install termination signal handler: {error}"),
                );
            }
        }
    });
}
