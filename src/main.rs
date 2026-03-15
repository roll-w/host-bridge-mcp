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

use application::config_reload::{spawn_config_reloader, ConfigReloadParticipant};
use application::execution_service::ExecutionService;
use application::operator_console::{ConsoleLogLevel, OperatorConsole};
use application::shutdown_controller::ShutdownController;
use cli::{help_text, parse_args, version_text};
use config::AppConfig;
use domain::platform::signal::wait_for_termination_signal;
use std::fmt;
use std::io;
use std::process::ExitCode;
use std::sync::Arc;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::field::Visit;
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::{fmt as tracing_fmt, util::SubscriberInitExt, EnvFilter, Layer};
use transport::mcp_streamable_http::{router, RequestAuthController};
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

    let config_path = AppConfig::resolve_config_path(cli_options.config_path.as_deref());
    let load_result = AppConfig::load_from_resolved_path(&config_path);

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

    let shutdown_controller = ShutdownController::default();
    let tui_active = tui::start(operator_console.clone(), shutdown_controller.clone());
    init_logging(operator_console.clone(), !tui_active);

    if tui_active {
        tracing::info!("Interactive TUI ready");
    } else {
        tracing::warn!(
            "Interactive TUI unavailable; confirmation-required commands will be rejected"
        );
    }

    spawn_system_signal_handler(shutdown_controller.clone());

    let auth_controller = match RequestAuthController::new(&config.server.access) {
        Ok(controller) => controller,
        Err(error) => {
            tracing::error!(error = %error, "Failed to initialize request authentication");
            return ExitCode::FAILURE;
        }
    };
    let execution_service = ExecutionService::new(config.clone());
    let app = router(
        execution_service.clone(),
        operator_console.clone(),
        auth_controller.clone(),
    );
    let reload_participants: Vec<Box<dyn ConfigReloadParticipant>> = vec![
        Box::new(operator_console.clone()),
        Box::new(auth_controller.clone()),
        Box::new(execution_service.clone()),
    ];
    spawn_config_reloader(
        config_path,
        (*config).clone(),
        reload_participants,
        shutdown_controller.clone(),
    );
    let bind_address = &config.server.bind_address;
    let listener = match bind_server_listener(bind_address).await {
        Ok(listener) => listener,
        Err(error) => {
            let message = format_bind_error(bind_address, &error);
            tracing::error!("{message}");
            return ExitCode::FAILURE;
        }
    };

    tracing::info!(
        bind_address = %bind_address,
        "host-bridge-mcp listening"
    );
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
            "Failed to bind {bind_address}: the port is already in use. Stop the other process or change `server.address` in the config."
        );
    }

    format!("Failed to bind {bind_address}: {error}")
}

fn init_logging(operator_console: OperatorConsole, mirror_to_stderr: bool) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::registry()
        .with(filter)
        .with(OperatorConsoleLayer { operator_console })
        .with(mirror_to_stderr.then(|| {
            tracing_fmt::layer()
                .with_timer(tracing_fmt::time::SystemTime::default())
                .with_target(false)
                .with_writer(io::stderr)
        }))
        .init();
}

struct OperatorConsoleLayer {
    operator_console: OperatorConsole,
}

impl<S> Layer<S> for OperatorConsoleLayer
where
    S: Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let level = match *event.metadata().level() {
            Level::ERROR => ConsoleLogLevel::Error,
            Level::WARN => ConsoleLogLevel::Warn,
            Level::INFO | Level::DEBUG | Level::TRACE => ConsoleLogLevel::Info,
        };

        let mut visitor = EventFieldVisitor::default();
        event.record(&mut visitor);
        self.operator_console.push_log(level, visitor.finish());
    }
}

#[derive(Default)]
struct EventFieldVisitor {
    message: Option<String>,
    fields: Vec<String>,
}

impl EventFieldVisitor {
    fn finish(self) -> String {
        match (self.message, self.fields.is_empty()) {
            (Some(message), true) => message,
            (Some(message), false) => format!("{message} {}", self.fields.join(" ")),
            (None, false) => self.fields.join(" "),
            (None, true) => String::new(),
        }
    }
}

impl Visit for EventFieldVisitor {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_string());
            return;
        }

        self.fields.push(format!("{}={value}", field.name()));
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn fmt::Debug) {
        let rendered = format!("{value:?}");
        if field.name() == "message" {
            self.message = Some(rendered.trim_matches('"').to_string());
            return;
        }

        self.fields.push(format!("{}={rendered}", field.name()));
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
        assert!(message.contains("server.address"));
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

fn spawn_system_signal_handler(shutdown_controller: ShutdownController) {
    tokio::spawn(async move {
        match wait_for_termination_signal().await {
            Ok(signal_name) => {
                tracing::warn!(signal = %signal_name, "System signal received. Shutting down server");
                let _ = shutdown_controller.request_shutdown();
            }
            Err(error) => {
                tracing::error!(error = %error, "Failed to install termination signal handler");
            }
        }
    });
}
