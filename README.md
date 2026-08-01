<div align="center">

# Host Bridge MCP

Execute local or remote host commands through MCP with policy checks, optional TUI approval, live output, and persisted
results.

[Quickstart](#quickstart) | [Usage](#usage) | [Configuration](#configuration) | [TUI](#tui) | [Security](#security)

</div>

`host-bridge-mcp` is a Rust MCP server for environments where the MCP client and the command toolchain are not running
in the same place. It can execute commands on the local host or on configured SSH targets, while keeping policy
decisions, operator approval, output streaming, and path translation in one service.

Each tool call carries one command string. Normal commands are parsed into an executable and arguments. A command string
containing shell operators is always confirmation-gated; local execution uses the system shell after approval.

> [!WARNING]
> This server can execute commands on configured hosts and SSH targets. Keep it on a trusted network, bind it to
`127.0.0.1` unless you have a specific reason not to, and review the execution policy before use.

## Features

- Execute commands on the implicit local `host` target or on named SSH targets.
- Apply `allow`, `confirm`, or `deny` policies globally, per command, and per subcommand prefix.
- Require a local operator to approve high-risk commands in the TUI.
- Stream merged stdout and stderr through MCP logging notifications and per-execution SSE.
- Return structured execution results with an `executionId`, exit status, and output.
- Persist the raw merged output for each execution under the host-bridge data directory.
- Translate executable paths, argument paths, and working directories across host environments.
- Watch the configuration file and apply supported changes without restarting the process.
- Expose `execute_command` and `get_execution_environment` through streamable HTTP MCP.

## Quickstart

### Requirements

A Rust toolchain with `cargo` is required.

### Build

```bash
cargo build --release
```

The binary is written to `./target/release/host-bridge-mcp`.

### Configure

Use [`host-bridge.yaml`](host-bridge.yaml) as the starting point. Configuration is resolved in this order, from highest
to lowest priority:

1. `--config <PATH>`
2. `HOST_BRIDGE_CONFIG`
3. `host-bridge.yaml` in the current working directory

If the default `host-bridge.yaml` does not exist, the server uses built-in defaults. A missing file supplied explicitly
with `--config` or `HOST_BRIDGE_CONFIG` is an error.

### Run

```bash
./target/release/host-bridge-mcp --config host-bridge.yaml
```

The default listener is `127.0.0.1:8787`. When standard input and output are attached to a terminal, the process also
starts the local TUI.

Useful CLI flags:

| Flag                  | Description                        |
|-----------------------|------------------------------------|
| `-c, --config <PATH>` | Use a custom configuration file.   |
| `-h, --help`          | Show help text and exit.           |
| `-V, --version`       | Print the server version and exit. |

## Usage

### HTTP endpoints

| Endpoint                                                | Purpose                       | Authentication                                           |
|---------------------------------------------------------|-------------------------------|----------------------------------------------------------|
| `http://127.0.0.1:8787/mcp`                             | Streamable HTTP MCP endpoint. | Required when `server.access.api-key-env` is configured. |
| `http://127.0.0.1:8787/health`                          | Returns `{ "status": "ok" }`. | Always public.                                           |
| `http://127.0.0.1:8787/executions/<executionId>/stream` | Per-execution SSE stream.     | Required when `server.access.api-key-env` is configured. |

### MCP client configuration

Minimal client entry:

```json
{
  "type": "streamable-http",
  "url": "http://127.0.0.1:8787/mcp"
}
```

To protect the MCP endpoint and execution streams, configure the name of an environment variable that contains the key.
The value of `api-key-env` is the variable name, not the secret itself:

```yaml
server:
  access:
    api-key-env: HOST_BRIDGE_API_KEY
```

Set the secret in the server process environment:

```bash
export HOST_BRIDGE_API_KEY='replace-with-a-long-random-value'
```

The client must then send the same value as a fixed bearer token:

```json
{
  "type": "streamable-http",
  "url": "http://127.0.0.1:8787/mcp",
  "headers": {
    "Authorization": "Bearer replace-with-a-long-random-value"
  }
}
```

The `/health` endpoint remains public when authentication is enabled.

### Tool: `execute_command`

Call MCP `tools/call` with the tool name `execute_command`.

| Argument           | Description                                                                              |
|--------------------|------------------------------------------------------------------------------------------|
| `command`          | Required command string. Quoted arguments are supported.                                 |
| `server`           | Optional execution target name. Defaults to `execution.default-server`, normally `host`. |
| `workingDirectory` | Optional working directory. Local paths must exist and be directories.                   |
| `timeoutMs`        | Optional timeout in milliseconds. Values above `execution.max-timeout-ms` are clamped.   |
| `env`              | Optional environment variables merged into the child process environment.                |
| `headLines`        | Optional number of leading output lines to return.                                       |
| `tailLines`        | Optional number of trailing output lines to return.                                      |
| `maxChars`         | Optional character limit applied after line filtering. `0` disables the limit.           |

Example:

```json
{
  "command": "cargo test --workspace",
  "server": "host",
  "workingDirectory": "/workspace/project",
  "timeoutMs": 600000,
  "env": {
    "RUST_LOG": "info"
  },
  "headLines": 40,
  "tailLines": 40,
  "maxChars": 12000
}
```

Tool behavior:

- A normal local command is started as a child process without shell chaining.
- Shell operators such as `&&`, `||`, `;`, `|`, or an unquoted newline always require TUI approval. After approval, the
  local command runs through the system shell.
- The effective policy is evaluated before execution. A `confirm` decision keeps the MCP call pending until the operator
  approves or rejects it.
- The call returns after execution finishes, times out, or fails to start.
- `headLines`, `tailLines`, and `maxChars` only shape the returned `output`; the persisted execution log remains
  unfiltered.

The final structured result has this shape:

```json
{
  "executionId": "123e4567-e89b-12d3-a456-426614174000",
  "status": "completed",
  "exit": {
    "code": 0,
    "success": true,
    "timedOut": false
  },
  "message": "Execution completed",
  "output": "..."
}
```

The returned output combines stdout and stderr in arrival order. The raw output is also written to
`executions/<executionId>.log` below the host-bridge data directory. Live execution records are temporary; persisted
output files remain until you remove them manually.

### Tool: `get_execution_environment`

Call `get_execution_environment` to discover the configured execution targets. The result includes the default target
name and each available target's name and platform. This is useful when an MCP client needs to choose a target before
calling `execute_command`.

### MCP logging notifications

While a request is waiting for approval or running, the server emits `logging/message` notifications. Their structured
payloads can include:

- `approval_pending`
- `approval_resolved`
- `status`
- `output`
- `exit`
- `error`
- `lagged`

### SSE execution stream

After obtaining an `executionId`, subscribe to its stream:

```bash
curl -N "http://127.0.0.1:8787/executions/<executionId>/stream"
```

The event names are `status`, `output`, `exit`, `error`, and `lagged`. Add the same `Authorization: Bearer <key>` header
used by the MCP client when request authentication is enabled.

## TUI

The TUI starts only when both standard input and standard output are attached to an interactive terminal. It provides:

- pending approval requests and their execution details
- live server logs
- historical logs from the beginning of the process
- log selection and terminal clipboard copying

| Key or gesture                | Action                                                         |
|-------------------------------|----------------------------------------------------------------|
| `Up` / `Down`                 | Select a pending approval.                                     |
| `a`                           | Approve the selected request.                                  |
| `r`                           | Reject the selected request.                                   |
| Mouse wheel / `PgUp` / `PgDn` | Scroll logs vertically.                                        |
| `Left` / `Right`              | Scroll logs horizontally.                                      |
| `Home`                        | Jump to the beginning of the log.                              |
| `End`                         | Return to the live log tail.                                   |
| Mouse drag                    | Copy the selected visible log lines to the terminal clipboard. |
| `q` / `Q`                     | Request graceful shutdown.                                     |

If no interactive TTY is available, confirmation-required commands are rejected. They are never silently executed
without approval.

The server also handles `SIGINT` and `SIGTERM` on Unix and `Ctrl+C` on non-Unix platforms. Shutdown stops accepting new
work and exits gracefully.

## Configuration

The configuration file uses YAML and rejects unknown keys. The repository's [`host-bridge.yaml`](host-bridge.yaml)
contains an annotated example. The sections below explain the settings that affect normal deployments.

### Minimal configuration

```yaml
server:
  address: 127.0.0.1:8787

logging:
  memory-buffer-lines: 2000
  persist-file: false

execution:
  default-action: confirm
  default-timeout-ms: 1800000
  max-timeout-ms: 7200000
```

These are also the built-in defaults except that an explicit `server.address` is shown for clarity. `persist-file`
defaults to `false`.

### Reload behavior

The server watches the resolved configuration file after startup. Valid changes to execution policy, execution targets,
authentication, and logging are applied without restarting the process. A change to `server.address` is recorded, but
the listener keeps its current address until restart. Invalid or unreadable updates are rejected and the previous
configuration remains active.

### `server`: listener and request authentication

- `server.address` controls the HTTP bind address. The default is `127.0.0.1:8787`. Changing it requires a restart to
  take effect.
- `server.access.api-key-env` is optional. It names the environment variable containing the expected API key.
- When `api-key-env` is set, the variable must exist and must not be empty. Clients must send
  `Authorization: Bearer <value>` to `/mcp` and `/executions/<executionId>/stream`.
- `/health` does not require authentication.

### `logging`: TUI history and log files

- `memory-buffer-lines` controls how many recent lines are kept in memory for fast TUI access. It must be greater than
  zero.
- `file-path` optionally selects an explicit backing log file.
- `persist-file: true` keeps the current backing log file. If an existing file is found on startup, it is archived with
  a date suffix before a new current log is opened.
- `persist-file: false` uses a temporary backing file when `file-path` is omitted and deletes that file on exit.
- When `file-path` is omitted, backing logs live under the host-bridge data directory in `logs/`.

The data directory is usually `~/.host-bridge-mcp` on Unix-like hosts. If that location is unavailable, the server tries
a `.host-bridge-mcp` directory beside the executable and then one below the current working directory. Execution output
is stored separately under `executions/` and is retained until manually removed.

### `execution`: defaults and timeouts

- `default-action` is the fallback policy for commands without a matching command rule: `allow`, `confirm`, or `deny`.
  The default is `confirm`.
- `default-working-directory` supplies a working directory when a request and its matching policy do not provide one.
- `default-server` selects the execution target when `execute_command.server` is omitted. The implicit local target is
  named `host`.
- `default-timeout-ms` sets the default request timeout. It must be greater than zero; the default is 30 minutes.
- `max-timeout-ms` caps request-provided timeouts and must be at least `default-timeout-ms`. The default is 2 hours.
- `target-platform` selects the platform used for the implicit local `host` target: `auto`, `windows`, `linux`, or
  `macos`.
- `path-mappings` defines path translation rules for the implicit local target.

### `execution.commands`: policy rules

Group each executable with a default action and optional subcommand overrides:

```yaml
execution:
  default-action: confirm
  commands:
    - command: cargo
      action: allow
      rules:
        - args-prefix:
            - publish
          action: confirm

    - command: npm
      action: allow
      rules:
        - args-prefix:
            - publish
          action: deny

    - command: mvn
      action: allow
      rules:
        - args-prefix:
            - clean
            - install
          action: confirm
```

Matching works as follows:

1. The executable name is normalized, including its path and common Windows suffixes such as `.exe`, `.cmd`, `.bat`, and
   `.ps1`.
2. Each `args-prefix` is matched against the leading normalized argument tokens.
3. The most specific matching prefix wins.
4. If multiple rules have the same specificity, the later rule wins.
5. If no rule matches, `execution.default-action` is used.

Shell operators are an additional safety boundary: a command containing them is confirmation-gated regardless of the
configured command action.

### `execution.servers`: local and SSH targets

The local `host` target is always available. Add named targets when commands need to run on another machine:

```yaml
execution:
  default-server: host
  servers:
    - name: build-linux
      transport: ssh
      host: build.example.com
      port: 22
      user: builder
      target-platform: linux
      auth:
        type: agent
      known-hosts-file: /home/builder/.ssh/known_hosts
      connection-idle-timeout-ms: 300000
      path-mappings:
        - from: /workspace
          to: /srv/workspace
```

Set `execute_command.server` to `build-linux` to use this target. An SSH target must have an explicit `target-platform`:
`windows`, `linux`, or `macos`. The name `host` is reserved for the local transport.

Supported SSH authentication modes:

| `auth.type`     | `auth.ref`                                               |
|-----------------|----------------------------------------------------------|
| `agent`         | Omit `ref`; use the configured SSH agent.                |
| `identity-file` | Path to the private identity file.                       |
| `password-env`  | Name of an environment variable containing the password. |
| `password-file` | Path to a file containing the password.                  |

Do not put passwords directly in YAML. `known-hosts-file` is optional, but omitting it disables strict host key
verification for that target.

### `execution.path-mappings`: cross-environment paths

Mappings replace a matching `from` prefix with `to`. They are applied to executable paths, path-like arguments, and
configured working directories. Rules are evaluated in order, and the first matching applicable rule wins.

```yaml
execution:
  target-platform: windows
  path-mappings:
    - from: /workspace/mnt/d
      to: "D:\\"
      platforms:
        - windows
        - wsl
```

The optional `platforms` list accepts `windows`, `linux`, `macos`, and `wsl`. The `wsl` selector matches whether the
server process is running inside WSL; it is not an SSH target platform. An omitted list makes the rule
platform-agnostic. Use target-specific `path-mappings` under an entry in `execution.servers` when different execution
environments need different translations.

## Security

Recommended baseline:

- Bind to `127.0.0.1` or another explicitly trusted interface. The API key is a bearer credential, not transport
  encryption.
- Keep `execution.default-action: confirm` unless the allow-list is deliberately narrow and reviewed.
- Deny high-impact commands such as publish, deploy, release, and destructive cleanup flows unless they have a clear
  approval process.
- Treat shell operators as high risk; they always require TUI approval, and local execution uses a shell after approval.
- Keep the TUI attached when using confirmation-based policies. A headless process rejects confirmation-required
  commands.
- Run as a non-root user where possible.
- Prefer `known-hosts-file` for SSH targets so host key verification remains enabled.
- Protect the host-bridge data directory. Logs, execution output, approval details, and command environment values may
  contain sensitive data.

## Further reading

- [`host-bridge.yaml`](host-bridge.yaml): annotated configuration example.
- [`docs/usage.md`](docs/usage.md): runtime flow and protocol details.
- [`DESIGN.md`](DESIGN.md): architecture and execution flow.

## License

Licensed under the Apache License, Version 2.0. See [`LICENSE`](LICENSE).
