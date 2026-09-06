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

use application::browser;
use application::config_reload::{ConfigReloadParticipant, spawn_config_reloader};
use application::data_dir::DataDirectory;
use application::execution_service::ExecutionService;
use application::operator_console::{ConsoleLogLevel, OperatorConsole};
use application::shutdown_controller::ShutdownController;
use cli::{parse_args, resolve_bind_address};
use config::AppConfig;
use domain::platform::signal::wait_for_termination_signal;
use std::fmt;
use std::io;
use std::process::ExitCode;
use std::sync::Arc;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::field::Visit;
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::{EnvFilter, Layer, fmt as tracing_fmt, util::SubscriberInitExt};
use transport::auth::RequestAuthController;
use transport::http::{WebSessionController, router};
use transport::tui;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> ExitCode {
    let cli_options = match parse_args(std::env::args()) {
        Ok(options) => options,
        Err(error) => {
            let exit_code = error.exit_code();
            if error.use_stderr() {
                eprint!("{error}");
            } else {
                print!("{error}");
            }
            return ExitCode::from(exit_code as u8);
        }
    };

    let config_path = AppConfig::resolve_config_path(cli_options.config_path.as_deref());
    let load_result = AppConfig::load_from_resolved_path(&config_path);

    let mut loaded_config = match load_result {
        Ok(config) => config,
        Err(error) => {
            eprintln!("Failed to load config: {error}");
            return ExitCode::FAILURE;
        }
    };

    match resolve_bind_address(
        &loaded_config.server.bind_address,
        cli_options.host.as_deref(),
        cli_options.port,
    ) {
        Ok(bind_address) => loaded_config.server.bind_address = bind_address,
        Err(error) => {
            eprintln!("Invalid bind address override: {error}");
            return ExitCode::from(2);
        }
    }

    if let Some(tui) = cli_options.tui {
        loaded_config.tui = tui;
    }
    if let Some(web) = cli_options.web {
        loaded_config.web = web;
    }

    let config = Arc::new(loaded_config);

    let data_directory = match DataDirectory::new(config.data_dir.as_deref()) {
        Ok(data_directory) => data_directory,
        Err(error) => {
            eprintln!("Failed to initialize application data directory: {error}");
            return ExitCode::FAILURE;
        }
    };

    let operator_console = match OperatorConsole::with_data_directory(
        config.logging.clone(),
        data_directory.clone(),
    ) {
        Ok(console) => console,
        Err(error) => {
            eprintln!("Failed to initialize log storage: {error}");
            return ExitCode::FAILURE;
        }
    };

    let shutdown_controller = ShutdownController::default();
    let tui_active = tui::start(
        operator_console.clone(),
        shutdown_controller.clone(),
        config.tui,
    );
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
    let execution_service =
        match ExecutionService::try_new_with_data_directory(config.clone(), data_directory.clone())
        {
            Ok(service) => service,
            Err(error) => {
                tracing::error!(error = %error, "Failed to initialize execution service");
                return ExitCode::FAILURE;
            }
        };
    let web_session = WebSessionController::new();
    let app = router(
        execution_service.clone(),
        operator_console.clone(),
        auth_controller.clone(),
        config_path.clone(),
        config.clone(),
        web_session.clone(),
        data_directory,
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

    if config.web {
        let web_url = web_session.create_bootstrap_url(bind_address);
        if let Err(error) = browser::open(&web_url) {
            tracing::warn!(error = %error, "Failed to open the web console in a browser");
        } else {
            tracing::info!("Web console opened in the default browser");
        }
    }

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
        if event.metadata().target() == "host_bridge::command_output" {
            self.operator_console
                .push_command_output_log(level, visitor.finish());
        } else {
            self.operator_console.push_log(level, visitor.finish());
        }
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
