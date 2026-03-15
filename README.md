<div align="center">

# Host Bridge MCP

Run host commands through MCP with TUI approvals, live logs, and full final output.

[Quickstart](#quickstart) | [Configuration](#configuration) | [Usage](#usage) | [TUI](#tui) | [Security](#security)

</div>

`host-bridge-mcp` is a Rust MCP server that executes exactly one host command per tool call.
It is designed for setups where your MCP client runs in a different environment (container, WSL, remote workspace) but
still needs the local host toolchain.

> [!WARNING]
> This server can execute host commands.
> Do not expose it to untrusted networks. Review `host-bridge.yaml` carefully before use.

## Features

- Execute exactly one command line per tool call
- Block the MCP tool call until execution completes, then return merged command output
- Show live operational logs and pending approvals in a local TUI
- Require approval in the TUI when policy demands confirmation
- Keep a hot in-memory log buffer plus optional on-disk log backing for full log history
- Expose per-execution SSE streams for external subscribers
- Persist merged execution output files by `executionId`
- Support path mapping for container/WSL/host interoperability
- Use grouped command policies with concise subcommand overrides

## Quickstart

### Build

Prerequisite: a Rust toolchain with `cargo`

```bash
cargo build --release
```

Binary: `./target/release/host-bridge-mcp`

### Configure

Configuration load order (highest to lowest priority):

1. CLI `--config <PATH>`
2. `HOST_BRIDGE_CONFIG` environment variable
3. `host-bridge.yaml` in the working directory

Start from the default template: `host-bridge.yaml`.

### Run

```bash
./target/release/host-bridge-mcp --config host-bridge.yaml
```

Useful CLI flags:

- `-c, --config <PATH>`: use a custom config file
- `-h, --help`: show help text and exit
- `-V, --version`: print version and exit

By default the server listens on `127.0.0.1:8787`.

## TUI

When the process starts in an interactive terminal, it opens a TUI that shows:

- pending approval requests
- live server logs
- the full log history from head to tail

TUI keys:

- `Up` / `Down`: select approval entries
- `a`: approve the selected request
- `r`: reject the selected request
- mouse wheel: scroll logs
- drag inside the log panel: send the selected visible log lines to the terminal clipboard
- `PgUp` / `PgDn`: scroll logs
- `Home` / `End`: jump to the start or follow the tail
- `q`: gracefully shut down the server

If the process is not attached to a TTY, the TUI is disabled and confirmation-required commands are rejected.

The server also handles system termination signals and shuts down gracefully on `SIGINT` / `SIGTERM` (or `Ctrl+C` on
non-Unix platforms).

## Usage

### Connect to MCP

- MCP endpoint: `http://127.0.0.1:8787/mcp`
- Health check: `http://127.0.0.1:8787/health`

Example MCP client configuration:

```json
{
  "type": "streamable-http",
  "url": "http://localhost:8787/mcp"
}
```

If `server.access.api-key-env` is configured, the client must send a fixed
`Authorization: Bearer <key>` header.

Authenticated MCP client configuration:

```json
{
  "type": "streamable-http",
  "url": "http://localhost:8787/mcp",
  "headers": {
    "Authorization": "Bearer sk-example"
  }
}
```

Recommended setup:

1. Set the secret in the server environment, for example `HOST_BRIDGE_API_KEY`
2. Reference that environment variable in `host-bridge.yaml` with `server.access.api-key-env`
3. Configure the MCP client entry to send the same key in `Authorization`

### Tool: `execute_command`

Call MCP `tools/call` with tool name `execute_command`.

Example tool arguments:

```json
{
  "command": "mvn clean compile",
  "workingDirectory": "/workspace/project",
  "timeoutMs": 600000,
  "env": {
    "MAVEN_OPTS": "-Xmx1g"
  }
}
```

Tool behavior:

- `command` must contain exactly one command line
- shell chaining operators such as `&&`, `||`, `;`, and `|` are rejected
- if policy is `confirm`, the tool call stays pending until the local TUI operator approves or rejects it
- if approved, the command runs and the tool returns only after execution completes
- the final tool result includes `executionId`, final `status`, `exit`, and merged `output`
- the merged output is also written to the host-bridge data directory as `executions/<executionId>.log`
- persisted execution output files are retained until you remove them manually

### SSE execution stream

The server also exposes a per-execution SSE stream:

```bash
curl -N "http://127.0.0.1:8787/executions/<executionId>/stream"
```

Events include `status`, `output`, `exit`, and `error`.

### MCP logging notifications

While a tool call is pending or running, the server also emits MCP `logging/message` notifications.
These notifications include structured JSON for approval state, incremental output, exit status, and runtime errors.

## Configuration

Key sections in `host-bridge.yaml`:

- `server`
- `logging`
- `execution`
- `execution.commands`
- `execution.path-mappings`

Highlights:

- `server.address`: HTTP bind address (requires restart if changed at runtime)
- `server.access.api-key-env`: optional environment-variable-backed API key for fixed `Authorization: Bearer <key>`
  request authentication
- `logging.memory-buffer-lines`: hot in-memory log window for the TUI
- `logging.file-path`: optional explicit log file path
- `logging.persist-file`: keep or delete the backing log file on exit
- default persistent log path: host-bridge data directory + `logs/host-bridge-mcp.log`
- default temporary log path: host-bridge data directory + `logs/host-bridge-mcp-<uuid>.log`
- default execution output path: host-bridge data directory + `executions/<executionId>.log`
- the host-bridge data directory is usually `~/.host-bridge-mcp` on Unix-like hosts
- persistent logs append across restarts; execution output files accumulate until manually cleaned up
- `execution.default-action`: fallback action for unmatched commands
- `execution.commands`: grouped command policies with nested `rules` overrides
- `execution.default-timeout-ms` / `execution.max-timeout-ms`: timeout control

## Security

Recommended baseline:

- bind to `127.0.0.1` or another trusted interface unless you fully trust the network
- keep `execution.default-action = "confirm"` unless you have a strong allow-list
- explicitly deny high-risk commands such as publish/deploy flows
- run as a non-root user where possible
- keep the TUI attached when using confirmation-based policies

## License

The project is licensed under the Apache License, Version 2.0.

```text
Copyright 2026-present RollW

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

       http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
```
