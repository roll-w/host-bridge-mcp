# Host Bridge MCP

`host-bridge-mcp` lets an MCP client run one command at a time on the local host or on a configured SSH host. The server
applies a policy before execution and can ask for approval through the local TUI or web console.

> [!WARNING]
> This server can execute arbitrary host commands. Keep it bound to `127.0.0.1`, protect it with authentication when
> exposed, and review every `allow` policy.

## Quick start

Download the archive for your operating system and CPU architecture from the
[Releases](https://github.com/roll-w/host-bridge-mcp/releases) page. Available archives are:

- Linux x64: `host-bridge-mcp-linux-x64-gnu-<version>.tar.gz`
- Linux arm64: `host-bridge-mcp-linux-arm64-gnu-<version>.tar.gz`
- Windows x64/arm64: `host-bridge-mcp-windows-<arch>-<version>.zip`
- macOS x64/arm64: `host-bridge-mcp-darwin-<arch>-<version>.zip`

Extract the archive, copy/edit `host-bridge.yaml`, and start the server from the extracted directory:

```bash
./host-bridge-mcp --config host-bridge.yaml
```

On Windows PowerShell:

```powershell
.\host-bridge-mcp.exe --config .\host-bridge.yaml
```

The server starts the TUI and opens the web console in the default browser by default. These startup choices can also be
set with `tui` and `web` in `host-bridge.yaml`. Use CLI overrides when needed:

```text
host-bridge-mcp --host 127.0.0.1 --port 8810 --tui false --web true
```

`--tui` and `--web` accept an optional boolean value, so both `--tui false` and `--tui=false` are valid. A bare option
means `true`; when omitted, the value from the configuration file is used. With `--tui false`, logs are written to the
terminal. When `web` is enabled, confirmation-required commands can still be approved in the web console. If no approval
interface is open, the request waits until its timeout and then is rejected.

[`host-bridge.yaml`](host-bridge.yaml) is a copy-and-edit configuration template. Its defaults use the loopback address,
the local host, and `confirm` for commands that do not have a more specific policy.

Configuration path order:

1. `--config <path>`
2. `HOST_BRIDGE_CONFIG`
3. `host-bridge.yaml`

## Connect an MCP client

The default endpoint is:

```text
http://127.0.0.1:8810/mcp
```

The endpoint supports the current and legacy Streamable HTTP revisions. The deprecated HTTP+SSE transport is not
supported.

Example client configuration:

```json
{
  "mcpServers": {
    "host-bridge": {
      "url": "http://127.0.0.1:8810/mcp"
    }
  }
}
```

The connection only requires the endpoint above. Tool arguments are exposed through the MCP tool schema.

To enable a shared API key, set this in the configuration:

```yaml
server:
  access:
    api-key-env: HOST_BRIDGE_API_KEY
```

Then set `HOST_BRIDGE_API_KEY` in the server environment and send it as a bearer token. `/health` remains public; the
MCP and execution-stream endpoints require authentication when it is configured.

## Web console

Open [http://127.0.0.1:8810](http://127.0.0.1:8810) after the server starts. With the default `--web true`, the server
opens a one-time bootstrap URL automatically. Closing and reopening the browser keeps the session; restarting the server
requires signing in again when an API key is configured.

The console provides:

- visual configuration and advanced YAML editing;
- pending-command approval and request details;
- runtime log history with live updates;
- persisted execution history and command output.

The web console always shows in-page notifications. Use the bell control in the top bar to grant browser permission for
parallel desktop notifications. New approvals are delivered over SSE while the console is open; execution history
refreshes after commands finish without a completion notification.

The JSON API is under `/api/v1/`. Normal responses and errors use the same `{status, data}` envelope.

SSE endpoints are the exception because their event stream carries event-specific payloads.

## Configuration reference

All configuration keys use `kebab-case`. Omitted fields use the defaults below, and unknown fields are rejected.
[`host-bridge.yaml`](host-bridge.yaml) contains the same structure as a commented copy-and-edit template.

### Startup interfaces

| Key   | Default | Description                                                                                                        |
|-------|---------|--------------------------------------------------------------------------------------------------------------------|
| `tui` | `true`  | Starts the local approval TUI.                                                                                     |
| `web` | `true`  | Enables web approvals and opens the console at startup. The HTTP API and MCP endpoint remain available when false. |

The configuration file controls the defaults. A command-line `--tui` or `--web` override takes precedence for that
process; a bare option enables the interface and an explicit `false` disables it. Changes to these two settings require
a restart because they control startup behavior.

### `server`

| Key                         | Default          | Description                                                                                                   |
|-----------------------------|------------------|---------------------------------------------------------------------------------------------------------------|
| `server.address`            | `127.0.0.1:8810` | HTTP listen address. Keep it on loopback unless the network is trusted.                                       |
| `server.access.api-key-env` | unset            | Name of the environment variable containing the bearer token. Protects `/mcp` and execution streams when set. |

`/health` is public. Authenticated requests use `Authorization: Bearer <token>`.

### `data-dir`

| Key        | Default          | Description                                                                                      |
|------------|------------------|--------------------------------------------------------------------------------------------------|
| `data-dir` | `~/.host-bridge` | Directory for execution history, command output, runtime logs, and generated SSH password files. |

The path may be absolute or relative to the working directory. A leading `~/` is expanded from the current user's home
directory. Changing `data-dir` through the configuration editor takes effect after restarting the server.

### `logging`

| Key                      | Default | Description                                                                                                                  |
|--------------------------|---------|------------------------------------------------------------------------------------------------------------------------------|
| `logging.retention-days` | `30`    | Runtime logs rotate when the calendar date changes; archived files older than this are removed. `0` keeps them indefinitely. |

### `history`

| Key                      | Default | Description                                                                              |
|--------------------------|---------|------------------------------------------------------------------------------------------|
| `history.retention-days` | `30`    | Remove terminal execution records older than this many days; must be greater than zero.  |
| `history.max-records`    | `1000`  | Maximum number of terminal records kept; running records are retained separately.         |

### `execution`

| Key                                   | Default   | Description                                                                                                |
|---------------------------------------|-----------|------------------------------------------------------------------------------------------------------------|
| `execution.default-action`            | `confirm` | Fallback policy: `allow`, `confirm`, or `deny`.                                                            |
| `execution.default-server`            | `host`    | Target used when a request does not specify `server`.                                                      |
| `execution.target-platform`           | `auto`    | Local target platform: `auto`, `windows`, `linux`, or `macos`.                                             |
| `execution.default-timeout-ms`        | `1800000` | Default timeout for approval waits and command execution when none is supplied; must be greater than zero. |
| `execution.max-timeout-ms`            | `7200000` | Maximum request timeout; must be at least the default timeout.                                             |
| `execution.default-working-directory` | unset     | Working directory when neither the request nor its policy provides one.                                    |
| `execution.commands`                  | `[]`      | Command policy entries; see below.                                                                         |
| `execution.servers`                   | `[]`      | Additional local or SSH execution targets; see below.                                                      |

### Command policy fields

Each item in `execution.commands` has the following fields:

| Key                         | Required | Description                                                                         |
|-----------------------------|----------|-------------------------------------------------------------------------------------|
| `command`                   | yes      | Command name or a `*` pattern. A standalone `*` must be quoted in YAML.             |
| `action`                    | yes      | `allow`, `confirm`, or `deny`.                                                      |
| `targets`                   | no       | Target names where this policy applies; empty means every target, including `host`. |
| `default-working-directory` | no       | Policy-specific fallback working directory.                                         |
| `rules`                     | no       | More specific rules selected by an exact argument prefix.                           |

Each `rules` item contains `args-prefix` (a non-empty list of exact argument tokens), `action`, and an optional
`default-working-directory`. Wildcards are supported in `command`, not in `args-prefix`.

Example:

```yaml
execution:
  default-action: deny
  commands:
    - command: "*"
      action: allow
    - command: rm
      action: deny
    - command: "sudo*"
      action: confirm
    - command: cargo
      action: allow
      targets: [ host, build-linux ]
      rules:
        - args-prefix: [ publish ]
          action: confirm
```

`targets` accepts `host` or a configured `execution.servers.name`; an empty list applies to every target. Nested rules
inherit the command's target scope. There is no separate per-server policy section.

The default action is `confirm`. Exact command rules take precedence over wildcard rules, and `deny` cannot be
overridden by an operator interface. Shell operators and redirections also require approval. Treat a catch-all `allow`
rule as broad authority.

### Execution targets

Every item in `execution.servers` requires a unique `name` and `transport` (`host` or `ssh`). The name `host` is
reserved for the local target.

For `transport: host`:

| Key               | Default | Description                                                    |
|-------------------|---------|----------------------------------------------------------------|
| `name`            | —       | Target name; use `host` to override the local target platform. |
| `target-platform` | `auto`  | `auto`, `windows`, `linux`, or `macos`.                        |

For `transport: ssh`:

| Key                          | Default  | Description                                                         |
|------------------------------|----------|---------------------------------------------------------------------|
| `name`                       | —        | Target name referenced by `default-server` or a request's `server`. |
| `host`                       | —        | SSH host name or address.                                           |
| `port`                       | `22`     | SSH port; must be greater than zero.                                |
| `user`                       | —        | SSH user name.                                                      |
| `target-platform`            | —        | `windows`, `linux`, or `macos`; `auto` is not allowed for SSH.      |
| `auth.type`                  | `agent`  | `agent`, `identity-file`, `password-env`, or `password-file`.       |
| `auth.ref`                   | unset    | Required for the three non-agent auth types; omit for `agent`.      |
| `verify-host-key`            | `true`   | Verify the SSH server key against known hosts. Set `false` only when another trusted mechanism performs verification. |
| `known-hosts-file`           | unset    | Optional known-hosts file; when unset, the user's default SSH known-hosts file is used. |
| `connection-idle-timeout-ms` | `300000` | Idle SSH connection timeout; must be greater than zero.             |

Working-directory and command paths are interpreted by the selected target.

## Optional SSH targets

Add a target under `execution.servers` and select it with `server: <name>` when calling `execute_command`:

```yaml
execution:
  servers:
    - name: build
      transport: ssh
      host: build.example.com
      port: 22
      user: deploy
      target-platform: linux
      auth:
        type: agent
```

The template contains examples for all supported authentication types.

For `password-env`, configure the named environment variable before starting the server. For `password-file`, the web
console can write a local password file directly from the SSH target editor; the password is not saved into YAML or
runtime logs.

## Available tools

### `execute_command`

Runs one command line on the selected execution target. Optional parameters select the target, working directory,
environment, timeout, and output limits. Commands that require approval wait for a decision in the TUI or web console.
Execution output and exit status are returned in the completed tool response. The server does not advertise the
deprecated MCP Logging capability or send `notifications/message`; live output remains available in the web console.

The MCP `timeoutMs` argument overrides the configured default for that request; if omitted,
`execution.default-timeout-ms` is used. A request that is not approved before the timeout is rejected.
Explicitly rejected approval requests are retained in execution history with a `rejected` status and no output.

### `get_execution_environment`

Takes no arguments. Returns the default target and the configured execution targets with their platform types.

## Building from source

Requires a stable Rust toolchain, Cargo, Node.js, and npm. Install the frontend dependencies:

```bash
cd web
npm ci
cd ..
```

The release build runs the frontend build automatically and embeds `web/dist` into the executable.

```bash
cargo build --release --locked
```

## License

Licensed under the Apache License, Version 2.0. See [`LICENSE`](LICENSE).
