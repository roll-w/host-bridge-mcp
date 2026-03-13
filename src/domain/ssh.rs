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
use russh::ChannelMsg;
use russh::client;
use russh::keys::{
    Algorithm, EcdsaCurve, HashAlg, PrivateKeyWithHashAlg, PublicKey,
    agent::client::{AgentClient, AgentStream},
    check_known_hosts_path, load_secret_key,
};
use std::borrow::Cow;
use std::collections::HashMap;
use std::fs;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

mod command;
mod connection_pool;

use self::command::{build_remote_command, keepalive_interval_for};
use self::connection_pool::SshConnectionManager;

const SSH_COMMAND_QUEUE_CAPACITY: usize = 64;
const SSH_OUTPUT_QUEUE_CAPACITY: usize = 256;
const KEEPALIVE_FAILURE_THRESHOLD: usize = 3;

type DynamicAgentClient = AgentClient<Box<dyn AgentStream + Send + Unpin + 'static>>;
pub(super) type SshSessionHandle = client::Handle<SshClientHandler>;

#[derive(Clone)]
pub struct SshClient {
    command_tx: mpsc::Sender<WorkerCommand>,
}

struct SshWorkerClient {
    connection_manager: SshConnectionManager,
}

struct WorkerCommand {
    target: SshTarget,
    platform: RuntimePlatform,
    request: SshCommandRequest,
    event_tx: mpsc::Sender<WorkerEvent>,
}

enum WorkerEvent {
    Output(String),
    Finished(Result<SshCommandResult, SshError>),
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
    #[error("SSH auth env '{0}' is set but the environment variable is missing")]
    MissingAuthEnv(String),
    #[error("SSH auth env '{0}' resolved to an empty value")]
    EmptyAuthEnv(String),
    #[error("failed to read SSH auth file '{0}': {1}")]
    AuthFileRead(String, String),
    #[error("SSH auth file '{0}' resolved to an empty value")]
    EmptyAuthFile(String),
    #[error("failed to connect to SSH server {0}:{1}: {2}")]
    Connect(String, u16, String),
    #[error("failed to load known hosts from {0}: {1}")]
    KnownHostsLoad(String, String),
    #[error("SSH host key for {0}:{1} is not trusted by {2}")]
    HostVerification(String, u16, String),
    #[error("failed to authenticate SSH agent for {0}@{1}:{2}: {3}")]
    Agent(String, String, u16, String),
    #[error("failed to load SSH identity file '{0}': {1}")]
    IdentityLoad(String, String),
    #[error("SSH authentication failed for {0}@{1}:{2}: {3}")]
    Authentication(String, String, u16, String),
    #[error("failed to open SSH exec channel for {0}@{1}:{2}: {3}")]
    ChannelOpen(String, String, u16, String),
    #[error("failed to execute remote command on {0}@{1}:{2}: {3}")]
    CommandStart(String, String, u16, String),
    #[error("remote command timed out after {0} ms")]
    Timeout(u64),
}

#[derive(Debug, thiserror::Error)]
pub(super) enum ClientHandlerError {
    #[error(transparent)]
    Russh(#[from] russh::Error),
    #[error("failed to load known hosts from {0}: {1}")]
    KnownHostsLoad(String, String),
    #[error("SSH host key for {0}:{1} is not trusted by {2}")]
    HostVerification(String, u16, String),
}

#[derive(Debug, Clone)]
pub(super) struct SshClientHandler {
    host: String,
    port: u16,
    known_hosts_file: Option<String>,
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

enum SessionConnectError {
    Socket(std::io::Error),
    Handler(ClientHandlerError),
}

impl Default for SshClient {
    fn default() -> Self {
        Self::new()
    }
}

impl SshClient {
    pub fn new() -> Self {
        let (command_tx, command_rx) = mpsc::channel(SSH_COMMAND_QUEUE_CAPACITY);
        spawn_ssh_worker(command_rx);
        Self { command_tx }
    }

    pub async fn execute_command<F>(
        &self,
        target: SshTarget,
        platform: RuntimePlatform,
        request: SshCommandRequest,
        on_output: F,
    ) -> Result<SshCommandResult, SshError>
    where
        F: Fn(String) + Send + Sync + 'static,
    {
        let error_host = target.host.clone();
        let error_port = target.port;
        let (event_tx, mut event_rx) = mpsc::channel(SSH_OUTPUT_QUEUE_CAPACITY);
        self.command_tx
            .send(WorkerCommand {
                target,
                platform,
                request,
                event_tx,
            })
            .await
            .map_err(|_| {
                SshError::Connect(
                    error_host.clone(),
                    error_port,
                    "SSH worker is not running".to_string(),
                )
            })?;

        while let Some(event) = event_rx.recv().await {
            match event {
                WorkerEvent::Output(text) => on_output(text),
                WorkerEvent::Finished(result) => return result,
            }
        }

        Err(SshError::Connect(
            error_host,
            error_port,
            "SSH worker stopped before returning a result".to_string(),
        ))
    }
}

impl SshWorkerClient {
    fn new() -> Self {
        Self {
            connection_manager: SshConnectionManager::new(),
        }
    }

    async fn run(mut command_rx: mpsc::Receiver<WorkerCommand>) {
        let worker = Self::new();
        while let Some(command) = command_rx.recv().await {
            let result = worker
                .execute_command(
                    command.target,
                    command.platform,
                    command.request,
                    command.event_tx.clone(),
                )
                .await;
            let _ = command.event_tx.send(WorkerEvent::Finished(result)).await;
        }
    }

    async fn execute_command(
        &self,
        target: SshTarget,
        platform: RuntimePlatform,
        request: SshCommandRequest,
        event_tx: mpsc::Sender<WorkerEvent>,
    ) -> Result<SshCommandResult, SshError> {
        let mut allow_fresh_retry = true;
        let mut force_fresh_session = false;

        loop {
            let mut connection = if force_fresh_session {
                self.connection_manager
                    .checkout_fresh(target.clone())
                    .await?
            } else {
                self.connection_manager.checkout(target.clone()).await?
            };
            let reused_connection = connection.was_reused();

            match execute_command_with_connection(
                &mut connection,
                target.clone(),
                platform,
                request.clone(),
                event_tx.clone(),
            )
            .await
            {
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

fn spawn_ssh_worker(command_rx: mpsc::Receiver<WorkerCommand>) {
    std::thread::Builder::new()
        .name("host-bridge-ssh".to_string())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("ssh worker runtime should initialize");
            runtime.block_on(SshWorkerClient::run(command_rx));
        })
        .expect("ssh worker thread should start");
}

async fn execute_command_with_connection(
    connection: &mut crate::domain::ssh::connection_pool::SshConnectionLease,
    target: SshTarget,
    platform: RuntimePlatform,
    request: SshCommandRequest,
    event_tx: mpsc::Sender<WorkerEvent>,
) -> Result<SshCommandResult, CommandAttemptError> {
    let user = target.user.clone();
    let host = target.host.clone();
    let port = target.port;

    let mut channel = connection
        .handle()
        .channel_open_session()
        .await
        .map_err(|error| {
            connection.discard();
            CommandAttemptError {
                error: SshError::ChannelOpen(user.clone(), host.clone(), port, error.to_string()),
                stage: CommandAttemptStage::ConnectionSetup,
            }
        })?;

    let remote_command = build_remote_command(platform, &request);
    channel.exec(true, remote_command).await.map_err(|error| {
        connection.discard();
        CommandAttemptError {
            error: SshError::CommandStart(user.clone(), host.clone(), port, error.to_string()),
            stage: CommandAttemptStage::ConnectionSetup,
        }
    })?;

    let wait_result = tokio::time::timeout(Duration::from_millis(request.timeout_ms), async {
        let mut code = None;
        let mut request_confirmed = false;

        loop {
            let Some(message) = channel.wait().await else {
                break;
            };

            match message {
                ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. } => {
                    request_confirmed = true;
                    let _ = event_tx
                        .send(WorkerEvent::Output(
                            String::from_utf8_lossy(data.as_ref()).into_owned(),
                        ))
                        .await;
                }
                ChannelMsg::Success => {
                    request_confirmed = true;
                }
                ChannelMsg::Failure => {
                    return Err(CommandAttemptError {
                        error: SshError::CommandStart(
                            user.clone(),
                            host.clone(),
                            port,
                            "remote server rejected exec request".to_string(),
                        ),
                        stage: if request_confirmed {
                            CommandAttemptStage::Runtime
                        } else {
                            CommandAttemptStage::ConnectionSetup
                        },
                    });
                }
                ChannelMsg::ExitStatus { exit_status } => {
                    request_confirmed = true;
                    code = Some(exit_status as i32);
                }
                ChannelMsg::ExitSignal { .. } => {
                    request_confirmed = true;
                    code.get_or_insert(-1);
                }
                ChannelMsg::Close | ChannelMsg::Eof => {}
                _ => {}
            }
        }

        Ok(SshCommandResult {
            code: code.unwrap_or(-1),
            success: code == Some(0),
            timed_out: false,
        })
    })
    .await;

    match wait_result {
        Ok(result) => result,
        Err(_) => {
            connection.discard();
            let _ = channel.close().await;
            Err(CommandAttemptError {
                error: SshError::Timeout(request.timeout_ms),
                stage: CommandAttemptStage::Runtime,
            })
        }
    }
}

fn should_retry_with_fresh_session(
    reused_connection: bool,
    allow_fresh_retry: bool,
    stage: CommandAttemptStage,
) -> bool {
    reused_connection && allow_fresh_retry && stage == CommandAttemptStage::ConnectionSetup
}

pub(super) async fn create_authenticated_session(
    target: SshTarget,
) -> Result<SshSessionHandle, SshError> {
    let mut session = connect_session(target.clone()).await?;
    authenticate_session(&mut session, target).await?;
    Ok(session)
}

async fn connect_session(target: SshTarget) -> Result<SshSessionHandle, SshError> {
    let host = target.host.clone();
    let port = target.port;
    match connect_session_with_config(target.clone(), false).await {
        Ok(session) => Ok(session),
        Err(error) if should_retry_with_legacy_rsa_host_key(&error) => {
            connect_session_with_config(target, true)
                .await
                .map_err(|retry_error| map_session_connect_error(host, port, retry_error))
        }
        Err(error) => Err(map_session_connect_error(host, port, error)),
    }
}

async fn connect_session_with_config(
    target: SshTarget,
    prefer_legacy_rsa_host_key: bool,
) -> Result<SshSessionHandle, SessionConnectError> {
    let config = Arc::new(build_client_config(&target, prefer_legacy_rsa_host_key));
    let handler = SshClientHandler::new(target.clone());
    let address = format!("{}:{}", target.host, target.port);
    let socket = tokio::net::TcpStream::connect(address)
        .await
        .map_err(SessionConnectError::Socket)?;

    if config.nodelay {
        let _ = socket.set_nodelay(true);
    }

    client::connect_stream(config, socket, handler)
        .await
        .map_err(SessionConnectError::Handler)
}

fn build_client_config(target: &SshTarget, prefer_legacy_rsa_host_key: bool) -> client::Config {
    let mut config = client::Config::default();
    config.nodelay = true;
    config.inactivity_timeout = Some(target.connection_idle_timeout);
    config.keepalive_max = KEEPALIVE_FAILURE_THRESHOLD;
    if let Some(interval) = keepalive_interval_for(target.connection_idle_timeout) {
        config.keepalive_interval = Some(interval);
    }
    if prefer_legacy_rsa_host_key {
        config.preferred.key = Cow::Owned(legacy_rsa_host_key_preference());
    }
    config
}

fn legacy_rsa_host_key_preference() -> Vec<Algorithm> {
    vec![
        Algorithm::Ed25519,
        Algorithm::Ecdsa {
            curve: EcdsaCurve::NistP256,
        },
        Algorithm::Ecdsa {
            curve: EcdsaCurve::NistP384,
        },
        Algorithm::Ecdsa {
            curve: EcdsaCurve::NistP521,
        },
        Algorithm::Rsa { hash: None },
        Algorithm::Rsa {
            hash: Some(HashAlg::Sha512),
        },
        Algorithm::Rsa {
            hash: Some(HashAlg::Sha256),
        },
    ]
}

fn should_retry_with_legacy_rsa_host_key(error: &SessionConnectError) -> bool {
    matches!(
        error,
        SessionConnectError::Handler(ClientHandlerError::Russh(russh::Error::WrongServerSig))
    )
}

fn map_session_connect_error(host: String, port: u16, error: SessionConnectError) -> SshError {
    match error {
        SessionConnectError::Socket(error) => SshError::Connect(host, port, error.to_string()),
        SessionConnectError::Handler(error) => map_client_handler_error(host, port, error),
    }
}

async fn authenticate_session(
    session: &mut SshSessionHandle,
    target: SshTarget,
) -> Result<(), SshError> {
    match target.auth.clone() {
        SshAuthTarget::Agent => authenticate_with_agent(session, target).await,
        SshAuthTarget::IdentityFile(path) => {
            authenticate_with_identity_file(session, target, path).await
        }
        SshAuthTarget::PasswordEnv(reference) => {
            let password = resolve_auth_env(&reference)?;
            authenticate_with_password(session, target, password).await
        }
        SshAuthTarget::PasswordFile(reference) => {
            let password = resolve_auth_file(&reference)?;
            authenticate_with_password(session, target, password).await
        }
    }
}

async fn authenticate_with_password(
    session: &mut SshSessionHandle,
    target: SshTarget,
    password: String,
) -> Result<(), SshError> {
    let user = target.user.clone();
    let host = target.host.clone();
    let port = target.port;
    let auth_result = session
        .authenticate_password(target.user.clone(), password)
        .await
        .map_err(|error| SshError::Authentication(user, host, port, error.to_string()))?;

    ensure_authentication_success(&target, auth_result.success())
}

async fn authenticate_with_identity_file(
    session: &mut SshSessionHandle,
    target: SshTarget,
    path: String,
) -> Result<(), SshError> {
    let user = target.user.clone();
    let host = target.host.clone();
    let port = target.port;
    let private_key = load_secret_key(&path, None)
        .map_err(|error| SshError::IdentityLoad(path.clone(), error.to_string()))?;
    let hash_alg = best_supported_rsa_hash(
        session,
        matches!(private_key.algorithm(), Algorithm::Rsa { .. }),
    )
    .await
    .map_err(|error| {
        SshError::Authentication(user.clone(), host.clone(), port, error.to_string())
    })?;
    let auth_result = session
        .authenticate_publickey(
            target.user.clone(),
            PrivateKeyWithHashAlg::new(Arc::new(private_key), hash_alg),
        )
        .await
        .map_err(|error| SshError::Authentication(user, host, port, error.to_string()))?;

    ensure_authentication_success(&target, auth_result.success())
}

async fn authenticate_with_agent(
    session: &mut SshSessionHandle,
    target: SshTarget,
) -> Result<(), SshError> {
    let user = target.user.clone();
    let host = target.host.clone();
    let port = target.port;
    let mut agent = connect_to_agent()
        .await
        .map_err(|error| SshError::Agent(user.clone(), host.clone(), port, error.to_string()))?;
    let identities = agent
        .request_identities()
        .await
        .map_err(|error| SshError::Agent(user.clone(), host.clone(), port, error.to_string()))?;

    if identities.is_empty() {
        return Err(SshError::Authentication(
            user.clone(),
            host.clone(),
            port,
            "SSH agent returned no identities".to_string(),
        ));
    }

    for identity in identities {
        let hash_alg = best_supported_rsa_hash(
            session,
            matches!(identity.algorithm(), Algorithm::Rsa { .. }),
        )
        .await
        .map_err(|error| {
            SshError::Authentication(user.clone(), host.clone(), port, error.to_string())
        })?;
        let auth_result = session
            .authenticate_publickey_with(target.user.clone(), identity, hash_alg, &mut agent)
            .await
            .map_err(|error| {
                SshError::Authentication(user.clone(), host.clone(), port, error.to_string())
            })?;

        if auth_result.success() {
            return Ok(());
        }
    }

    Err(SshError::Authentication(
        user,
        host,
        port,
        "authentication did not complete".to_string(),
    ))
}

async fn best_supported_rsa_hash(
    session: &SshSessionHandle,
    needs_hash: bool,
) -> Result<Option<HashAlg>, russh::Error> {
    if !needs_hash {
        return Ok(None);
    }

    session
        .best_supported_rsa_hash()
        .await
        .map(|hash| hash.flatten())
}

fn ensure_authentication_success(target: &SshTarget, success: bool) -> Result<(), SshError> {
    if success {
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

#[cfg(unix)]
async fn connect_to_agent() -> Result<DynamicAgentClient, russh::keys::Error> {
    Ok(AgentClient::connect_env().await?.dynamic())
}

#[cfg(windows)]
async fn connect_to_agent() -> Result<DynamicAgentClient, russh::keys::Error> {
    Ok(AgentClient::connect_pageant().await?.dynamic())
}

fn map_client_handler_error(host: String, port: u16, error: ClientHandlerError) -> SshError {
    match error {
        ClientHandlerError::Russh(russh::Error::WrongServerSig) => SshError::Connect(
            host,
            port,
            "server host key signature verification failed before known_hosts validation"
                .to_string(),
        ),
        ClientHandlerError::Russh(error) => SshError::Connect(host, port, error.to_string()),
        ClientHandlerError::KnownHostsLoad(path, reason) => SshError::KnownHostsLoad(path, reason),
        ClientHandlerError::HostVerification(host, port, path) => {
            SshError::HostVerification(host, port, path)
        }
    }
}

impl SshClientHandler {
    fn new(target: SshTarget) -> Self {
        Self {
            host: target.host,
            port: target.port,
            known_hosts_file: target.known_hosts_file,
        }
    }
}

impl client::Handler for SshClientHandler {
    type Error = ClientHandlerError;

    async fn check_server_key(
        &mut self,
        server_public_key: &PublicKey,
    ) -> Result<bool, Self::Error> {
        let Some(known_hosts_path) = self.known_hosts_file.clone() else {
            return Ok(true);
        };

        match check_known_hosts_path(&self.host, self.port, server_public_key, &known_hosts_path) {
            Ok(true) => Ok(true),
            Ok(false) => Err(ClientHandlerError::HostVerification(
                self.host.clone(),
                self.port,
                known_hosts_path,
            )),
            Err(russh::keys::Error::KeyChanged { .. }) => {
                Err(ClientHandlerError::HostVerification(
                    self.host.clone(),
                    self.port,
                    known_hosts_path,
                ))
            }
            Err(error) => Err(ClientHandlerError::KnownHostsLoad(
                known_hosts_path,
                error.to_string(),
            )),
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_SERVER_PUBLIC_KEY: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAILM+rvN+ot98qgEN796jTiQfZfG1KaT0PtFDJ/XFSqti user@example.com";

    fn sample_server_public_key() -> PublicKey {
        PublicKey::from_openssh(SAMPLE_SERVER_PUBLIC_KEY)
            .expect("sample server public key should parse")
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

    #[test]
    fn retry_legacy_rsa_host_key_only_for_wrong_server_signature() {
        assert!(should_retry_with_legacy_rsa_host_key(
            &SessionConnectError::Handler(ClientHandlerError::Russh(russh::Error::WrongServerSig))
        ));
        assert!(!should_retry_with_legacy_rsa_host_key(
            &SessionConnectError::Handler(ClientHandlerError::Russh(
                russh::Error::ConnectionTimeout
            ))
        ));
        assert!(!should_retry_with_legacy_rsa_host_key(
            &SessionConnectError::Handler(ClientHandlerError::KnownHostsLoad(
                "/home/dev/.ssh/known_hosts".to_string(),
                "invalid format".to_string()
            ))
        ));
        assert!(!should_retry_with_legacy_rsa_host_key(
            &SessionConnectError::Socket(std::io::Error::other("connection refused"))
        ));
    }

    #[test]
    fn legacy_rsa_host_key_preference_prioritizes_ssh_rsa_before_rsa_sha2() {
        let algorithms = legacy_rsa_host_key_preference();
        let ssh_rsa_index = algorithms
            .iter()
            .position(|algorithm| matches!(algorithm, Algorithm::Rsa { hash: None }))
            .expect("ssh-rsa should be present");
        let rsa_sha512_index = algorithms
            .iter()
            .position(|algorithm| {
                matches!(
                    algorithm,
                    Algorithm::Rsa {
                        hash: Some(HashAlg::Sha512)
                    }
                )
            })
            .expect("rsa-sha2-512 should be present");
        let rsa_sha256_index = algorithms
            .iter()
            .position(|algorithm| {
                matches!(
                    algorithm,
                    Algorithm::Rsa {
                        hash: Some(HashAlg::Sha256)
                    }
                )
            })
            .expect("rsa-sha2-256 should be present");

        assert!(ssh_rsa_index < rsa_sha512_index);
        assert!(ssh_rsa_index < rsa_sha256_index);
    }

    #[tokio::test]
    async fn check_server_key_accepts_any_key_when_known_hosts_is_unset() {
        let mut handler = SshClientHandler {
            host: "server.example.com".to_string(),
            port: 22,
            known_hosts_file: None,
        };

        let accepted = <SshClientHandler as client::Handler>::check_server_key(
            &mut handler,
            &sample_server_public_key(),
        )
        .await
        .expect("missing known_hosts_file should skip verification");

        assert!(accepted);
    }

    #[tokio::test]
    async fn check_server_key_uses_configured_known_hosts_path() {
        let missing_path = std::env::temp_dir().join(format!(
            "host-bridge-mcp-missing-known-hosts-{}",
            uuid::Uuid::new_v4()
        ));
        let missing_path = missing_path.to_string_lossy().to_string();
        let mut handler = SshClientHandler {
            host: "server.example.com".to_string(),
            port: 22,
            known_hosts_file: Some(missing_path.clone()),
        };

        let error = <SshClientHandler as client::Handler>::check_server_key(
            &mut handler,
            &sample_server_public_key(),
        )
        .await
        .expect_err("configured known_hosts_file should be consulted");

        match error {
            ClientHandlerError::KnownHostsLoad(path, _)
            | ClientHandlerError::HostVerification(_, _, path) => {
                assert_eq!(path, missing_path)
            }
            other => panic!("expected known_hosts verification error, got {other:?}"),
        }
    }

    #[test]
    fn wrong_server_signature_error_explains_known_hosts_is_not_involved() {
        let error = map_client_handler_error(
            "server.example.com".to_string(),
            22,
            ClientHandlerError::Russh(russh::Error::WrongServerSig),
        );

        match error {
            SshError::Connect(_, _, message) => {
                assert!(message.contains("before known_hosts validation"));
            }
            other => panic!("expected connect error, got {other:?}"),
        }
    }
}
