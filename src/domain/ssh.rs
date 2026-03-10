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

use crate::domain::execution_target::{SshAuthTarget, SshTarget};
use crate::domain::platform::runtime::RuntimePlatform;
use ssh2::{CheckResult, KnownHostFileKind, Session};
use std::collections::HashMap;
use std::fs;
use std::io::{self, Read};
use std::net::TcpStream;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

mod connection_pool;

use self::connection_pool::SshConnectionPool;

const SSH_POLL_INTERVAL: Duration = Duration::from_millis(20);

#[derive(Clone)]
pub struct SshClient {
    pool: Arc<SshConnectionPool>,
}

#[derive(Debug, Clone)]
pub struct SshCommandRequest {
    pub executable: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    pub working_directory: Option<String>,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshCommandResult {
    pub code: i32,
    pub success: bool,
    pub timed_out: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum SshError {
    #[error("ssh auth env '{0}' is set but the environment variable is missing")]
    MissingAuthEnv(String),
    #[error("ssh auth env '{0}' resolved to an empty value")]
    EmptyAuthEnv(String),
    #[error("failed to read ssh auth file '{0}': {1}")]
    AuthFileRead(String, String),
    #[error("ssh auth file '{0}' resolved to an empty value")]
    EmptyAuthFile(String),
    #[error("failed to connect to ssh server {0}:{1}: {2}")]
    Connect(String, u16, String),
    #[error("failed to initialize ssh session: {0}")]
    SessionInit(String),
    #[error("failed to perform ssh handshake with {0}:{1}: {2}")]
    Handshake(String, u16, String),
    #[error("failed to load known hosts from {0}: {1}")]
    KnownHostsLoad(String, String),
    #[error("ssh host key for {0}:{1} is not trusted by {2}")]
    HostVerification(String, u16, String),
    #[error("ssh authentication failed for {0}@{1}:{2}: {3}")]
    Authentication(String, String, u16, String),
    #[error("failed to open ssh exec channel for {0}@{1}:{2}: {3}")]
    ChannelOpen(String, String, u16, String),
    #[error("failed to execute remote command on {0}@{1}:{2}: {3}")]
    CommandStart(String, String, u16, String),
    #[error("failed while reading remote output from {0}@{1}:{2}: {3}")]
    Output(String, String, u16, String),
    #[error("remote command timed out after {0} ms")]
    Timeout(u64),
    #[error("failed to finalize remote command on {0}@{1}:{2}: {3}")]
    Finalize(String, String, u16, String),
}

impl Default for SshClient {
    fn default() -> Self {
        Self::new()
    }
}

impl SshClient {
    pub fn new() -> Self {
        Self {
            pool: Arc::new(SshConnectionPool::new()),
        }
    }

    pub fn execute_command<F>(
        &self,
        target: &SshTarget,
        platform: RuntimePlatform,
        request: &SshCommandRequest,
        on_output: F,
    ) -> Result<SshCommandResult, SshError>
    where
        F: FnMut(String),
    {
        let mut on_output = on_output;
        let mut allow_fresh_retry = true;
        let mut force_fresh_session = false;

        loop {
            let mut connection = if force_fresh_session {
                self.pool.checkout_fresh(target)?
            } else {
                self.pool.checkout(target)?
            };
            let reused_connection = connection.was_reused();

            match execute_command_with_connection(
                &mut connection,
                target,
                platform,
                request,
                &mut on_output,
            ) {
                Ok(result) => return Ok(result),
                Err(error)
                if should_retry_with_fresh_session(
                    reused_connection,
                    allow_fresh_retry,
                    error.stage,
                ) =>
                    {
                        allow_fresh_retry = false;
                        force_fresh_session = true;
                    }
                Err(error) => return Err(error.error),
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommandAttemptStage {
    ConnectionSetup,
    Runtime,
}

struct CommandAttemptError {
    error: SshError,
    stage: CommandAttemptStage,
}

fn execute_command_with_connection<F>(
    connection: &mut crate::domain::ssh::connection_pool::SshConnectionLease,
    target: &SshTarget,
    platform: RuntimePlatform,
    request: &SshCommandRequest,
    on_output: &mut F,
) -> Result<SshCommandResult, CommandAttemptError>
where
    F: FnMut(String),
{
    let session = connection.session().clone();

    let mut channel = session.channel_session().map_err(|error| {
        connection.discard();
        CommandAttemptError {
            error: SshError::ChannelOpen(
                target.user.clone(),
                target.host.clone(),
                target.port,
                error.to_string(),
            ),
            stage: CommandAttemptStage::ConnectionSetup,
        }
    })?;
    channel
        .handle_extended_data(ssh2::ExtendedData::Merge)
        .map_err(|error| {
            connection.discard();
            CommandAttemptError {
                error: SshError::ChannelOpen(
                    target.user.clone(),
                    target.host.clone(),
                    target.port,
                    error.to_string(),
                ),
                stage: CommandAttemptStage::ConnectionSetup,
            }
        })?;

    let remote_command = build_remote_command(platform, request);
    channel.exec(&remote_command).map_err(|error| {
        connection.discard();
        CommandAttemptError {
            error: SshError::CommandStart(
                target.user.clone(),
                target.host.clone(),
                target.port,
                error.to_string(),
            ),
            stage: CommandAttemptStage::ConnectionSetup,
        }
    })?;

    let deadline = Instant::now() + Duration::from_millis(request.timeout_ms);
    let mut buffer = [0u8; 8192];
    let mut timed_out = false;

    loop {
        let mut progressed = false;
        loop {
            match channel.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => {
                    progressed = true;
                    on_output(String::from_utf8_lossy(&buffer[..read]).into_owned());
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) => {
                    connection.discard();
                    return Err(CommandAttemptError {
                        error: SshError::Output(
                            target.user.clone(),
                            target.host.clone(),
                            target.port,
                            error.to_string(),
                        ),
                        stage: CommandAttemptStage::Runtime,
                    });
                }
            }
        }

        if channel.eof() {
            break;
        }

        if Instant::now() >= deadline {
            timed_out = true;
            connection.discard();
            let _ = channel.close();
            let _ = session.disconnect(None, "timeout", None);
            break;
        }

        if !progressed {
            std::thread::sleep(SSH_POLL_INTERVAL);
        }
    }

    if timed_out {
        return Err(CommandAttemptError {
            error: SshError::Timeout(request.timeout_ms),
            stage: CommandAttemptStage::Runtime,
        });
    }

    channel.wait_close().map_err(|error| {
        connection.discard();
        CommandAttemptError {
            error: SshError::Finalize(
                target.user.clone(),
                target.host.clone(),
                target.port,
                error.to_string(),
            ),
            stage: CommandAttemptStage::Runtime,
        }
    })?;

    let code = channel.exit_status().map_err(|error| {
        connection.discard();
        CommandAttemptError {
            error: SshError::Finalize(
                target.user.clone(),
                target.host.clone(),
                target.port,
                error.to_string(),
            ),
            stage: CommandAttemptStage::Runtime,
        }
    })?;

    Ok(SshCommandResult {
        code,
        success: code == 0,
        timed_out: false,
    })
}

fn should_retry_with_fresh_session(
    reused_connection: bool,
    allow_fresh_retry: bool,
    stage: CommandAttemptStage,
) -> bool {
    reused_connection && allow_fresh_retry && stage == CommandAttemptStage::ConnectionSetup
}

pub fn build_remote_command(platform: RuntimePlatform, request: &SshCommandRequest) -> String {
    match platform {
        RuntimePlatform::Windows => build_windows_remote_command(request),
        RuntimePlatform::Linux | RuntimePlatform::Macos => build_posix_remote_command(request),
    }
}

fn build_posix_remote_command(request: &SshCommandRequest) -> String {
    let env_prefix = build_posix_env_prefix(&request.env);
    let command = std::iter::once(quote_posix(&request.executable))
        .chain(request.args.iter().map(|value| quote_posix(value)))
        .collect::<Vec<_>>()
        .join(" ");
    let exec_command = if env_prefix.is_empty() {
        format!("exec {command}")
    } else {
        format!("exec env {env_prefix} {command}")
    };
    let script = match request.working_directory.as_deref() {
        Some(working_directory) => {
            format!("cd -- {} && {exec_command}", quote_posix(working_directory))
        }
        None => exec_command,
    };

    format!("sh -lc {}", quote_posix(&script))
}

fn build_windows_remote_command(request: &SshCommandRequest) -> String {
    let script = build_windows_script(request);
    let encoded = encode_powershell_command(&script);
    format!(
        "powershell -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -EncodedCommand {encoded}"
    )
}

fn build_posix_env_prefix(env: &HashMap<String, String>) -> String {
    let mut keys = env.keys().collect::<Vec<_>>();
    keys.sort();
    keys.into_iter()
        .filter_map(|key| {
            env.get(key)
                .map(|value| quote_posix(&format!("{key}={value}")))
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn build_windows_script(request: &SshCommandRequest) -> String {
    let mut lines = vec!["$ErrorActionPreference = 'Stop'".to_string()];
    if let Some(working_directory) = request.working_directory.as_deref() {
        lines.push(format!(
            "Set-Location -LiteralPath {}",
            quote_powershell(working_directory)
        ));
    }

    let mut keys = request.env.keys().collect::<Vec<_>>();
    keys.sort();
    for key in keys {
        if let Some(value) = request.env.get(key) {
            lines.push(format!(
                "[System.Environment]::SetEnvironmentVariable({}, {}, 'Process')",
                quote_powershell(key),
                quote_powershell(value)
            ));
        }
    }

    let command = std::iter::once(quote_powershell(&request.executable))
        .chain(request.args.iter().map(|value| quote_powershell(value)))
        .collect::<Vec<_>>()
        .join(" ");
    lines.push(format!("& {command}"));
    lines.push("exit $LASTEXITCODE".to_string());
    lines.join("\n")
}

fn authenticate_session(session: &Session, target: &SshTarget) -> Result<(), SshError> {
    let auth_result = match &target.auth {
        SshAuthTarget::Agent => session.userauth_agent(&target.user),
        SshAuthTarget::IdentityFile(path) => {
            session.userauth_pubkey_file(&target.user, None, Path::new(path), None)
        }
        SshAuthTarget::PasswordEnv(reference) => {
            let password = resolve_auth_env(reference)?;
            session.userauth_password(&target.user, &password)
        }
        SshAuthTarget::PasswordFile(reference) => {
            let password = resolve_auth_file(reference)?;
            session.userauth_password(&target.user, &password)
        }
    };

    auth_result.map_err(|error| {
        SshError::Authentication(
            target.user.clone(),
            target.host.clone(),
            target.port,
            error.to_string(),
        )
    })?;

    if session.authenticated() {
        Ok(())
    } else {
        Err(SshError::Authentication(
            target.user.clone(),
            target.host.clone(),
            target.port,
            "authentication did not complete".to_string(),
        ))
    }
}

fn verify_host_key(session: &Session, target: &SshTarget) -> Result<(), SshError> {
    let Some(known_hosts_path) = target.known_hosts_file.as_deref() else {
        return Ok(());
    };
    let (host_key, _) = session.host_key().ok_or_else(|| {
        SshError::HostVerification(
            target.host.clone(),
            target.port,
            known_hosts_path.to_string(),
        )
    })?;

    let mut known_hosts = session.known_hosts().map_err(|error| {
        SshError::KnownHostsLoad(known_hosts_path.to_string(), error.to_string())
    })?;
    known_hosts
        .read_file(Path::new(known_hosts_path), KnownHostFileKind::OpenSSH)
        .map_err(|error| {
            SshError::KnownHostsLoad(known_hosts_path.to_string(), error.to_string())
        })?;

    match known_hosts.check_port(&target.host, target.port, host_key) {
        CheckResult::Match => Ok(()),
        _ => Err(SshError::HostVerification(
            target.host.clone(),
            target.port,
            known_hosts_path.to_string(),
        )),
    }
}

fn connect_session(target: &SshTarget) -> Result<Session, SshError> {
    let address = format!("{}:{}", target.host, target.port);
    let tcp_stream = TcpStream::connect(&address)
        .map_err(|error| SshError::Connect(target.host.clone(), target.port, error.to_string()))?;
    tcp_stream
        .set_nonblocking(true)
        .map_err(|error| SshError::Connect(target.host.clone(), target.port, error.to_string()))?;

    let mut session = Session::new().map_err(|error| SshError::SessionInit(error.to_string()))?;
    session.set_tcp_stream(tcp_stream);
    session.handshake().map_err(|error| {
        SshError::Handshake(target.host.clone(), target.port, error.to_string())
    })?;
    session.set_blocking(false);
    Ok(session)
}

fn disconnect_session(session: Session) {
    let _ = session.disconnect(None, "idle timeout", None);
}

fn resolve_auth_env(reference: &str) -> Result<String, SshError> {
    let secret =
        std::env::var(reference).map_err(|_| SshError::MissingAuthEnv(reference.to_string()))?;
    if secret.trim().is_empty() {
        return Err(SshError::EmptyAuthEnv(reference.to_string()));
    }

    Ok(secret)
}

fn resolve_auth_file(reference: &str) -> Result<String, SshError> {
    let secret = fs::read_to_string(reference)
        .map_err(|error| SshError::AuthFileRead(reference.to_string(), error.to_string()))?;
    let secret = secret.trim_end_matches(['\r', '\n']).to_string();
    if secret.is_empty() {
        return Err(SshError::EmptyAuthFile(reference.to_string()));
    }

    Ok(secret)
}

fn quote_posix(value: &str) -> String {
    if value.is_empty() {
        return "''".to_string();
    }

    format!("'{}'", value.replace('\'', "'\\''"))
}

fn quote_powershell(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn encode_powershell_command(script: &str) -> String {
    let utf16 = script.encode_utf16().collect::<Vec<_>>();
    let mut bytes = Vec::with_capacity(utf16.len() * 2);
    for unit in utf16 {
        let [low, high] = unit.to_le_bytes();
        bytes.push(low);
        bytes.push(high);
    }

    encode_base64(&bytes)
}

fn encode_base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = *chunk.get(1).unwrap_or(&0);
        let third = *chunk.get(2).unwrap_or(&0);
        let combined = (u32::from(first) << 16) | (u32::from(second) << 8) | u32::from(third);

        encoded.push(TABLE[((combined >> 18) & 0x3f) as usize] as char);
        encoded.push(TABLE[((combined >> 12) & 0x3f) as usize] as char);
        if chunk.len() > 1 {
            encoded.push(TABLE[((combined >> 6) & 0x3f) as usize] as char);
        } else {
            encoded.push('=');
        }
        if chunk.len() > 2 {
            encoded.push(TABLE[(combined & 0x3f) as usize] as char);
        } else {
            encoded.push('=');
        }
    }

    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> SshCommandRequest {
        SshCommandRequest {
            executable: "cargo".to_string(),
            args: vec!["build".to_string(), "--release".to_string()],
            env: HashMap::from([("RUST_LOG".to_string(), "info debug".to_string())]),
            working_directory: Some("/srv/app".to_string()),
            timeout_ms: 3_000,
        }
    }

    #[test]
    fn posix_command_uses_shell_safe_wrapping() {
        let remote_command = build_remote_command(RuntimePlatform::Linux, &request());

        assert!(remote_command.contains("sh -lc"));
        assert!(remote_command.contains("/srv/app"));
        assert!(remote_command.contains("RUST_LOG=info debug"));
        assert!(remote_command.contains("cargo"));
        assert!(remote_command.contains("--release"));
    }

    #[test]
    fn windows_command_uses_encoded_powershell() {
        let mut request = request();
        request.executable = "cargo.exe".to_string();
        request.working_directory = Some("C:\\repo".to_string());
        let remote_command = build_remote_command(RuntimePlatform::Windows, &request);

        assert!(remote_command.starts_with(
            "powershell -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -EncodedCommand "
        ));
    }

    #[test]
    fn resolve_auth_env_reads_reference() {
        let target = SshTarget {
            host: "server.example.com".to_string(),
            port: 22,
            user: "deploy".to_string(),
            auth: SshAuthTarget::PasswordEnv("HOST_BRIDGE_TEST_SSH_PASSWORD".to_string()),
            known_hosts_file: None,
            connection_idle_timeout: Duration::from_secs(30),
        };

        unsafe {
            std::env::set_var("HOST_BRIDGE_TEST_SSH_PASSWORD", "secret");
        }
        let password = match &target.auth {
            SshAuthTarget::PasswordEnv(reference) => {
                resolve_auth_env(reference).expect("password should resolve")
            }
            _ => panic!("expected password env auth"),
        };
        unsafe {
            std::env::remove_var("HOST_BRIDGE_TEST_SSH_PASSWORD");
        }

        assert_eq!(password, "secret");
    }

    #[test]
    fn resolve_auth_file_reads_reference() {
        let file_path = std::env::temp_dir().join("host-bridge-mcp-ssh-password.txt");
        fs::write(&file_path, "secret\n").expect("secret file should be written");

        let password = resolve_auth_file(file_path.to_str().expect("path should be utf-8"))
            .expect("password file should resolve");

        let _ = fs::remove_file(&file_path);

        assert_eq!(password, "secret");
    }

    #[test]
    fn base64_encoder_matches_known_value() {
        assert_eq!(encode_base64(b"hello"), "aGVsbG8=");
    }

    #[test]
    fn retry_policy_only_retries_reused_connection_setup_failures_once() {
        assert!(should_retry_with_fresh_session(
            true,
            true,
            CommandAttemptStage::ConnectionSetup
        ));
        assert!(!should_retry_with_fresh_session(
            false,
            true,
            CommandAttemptStage::ConnectionSetup
        ));
        assert!(!should_retry_with_fresh_session(
            true,
            false,
            CommandAttemptStage::ConnectionSetup
        ));
        assert!(!should_retry_with_fresh_session(
            true,
            true,
            CommandAttemptStage::Runtime
        ));
    }
}
