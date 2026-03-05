<div align="center">

# Host Bridge MCP

Run host toolchain commands via MCP and stream output with SSE.

[Quickstart](#quickstart) | [Configuration](#configuration) | [Usage](#usage) | [Security](#security) | [Documentation](#documentation)

</div>

`host-bridge-mcp` is a Rust MCP server that executes a single command line on the machine where the server runs.
It returns an SSE URL you can subscribe to for real-time stdout/stderr and exit status.

This is useful when your MCP client runs in a different environment (container, WSL, remote) but you need access to the host toolchain.

> [!WARNING]
> This server can execute commands on the machine where it runs.
> Do not expose it to untrusted networks. Review and lock down policies in `host-bridge.toml`.

## Features

- Execute a single command string (for example, `mvn clean compile`)
- Stream output and exit status in real time via SSE
- Policy engine with `allow`, `confirm`, `deny` (command and subcommand level)
- Optional working directory and environment variables per request
- Path mapping rules for container/WSL/host interoperability

## Quickstart

### Build

Prerequisites: Rust toolchain (`cargo`) for building from source

```bash
cargo build --release
```

Binary: `./target/release/host-bridge-mcp`

### Configure

Configuration load order (highest to lowest priority):

1. CLI `--config <PATH>`
2. `HOST_BRIDGE_CONFIG` environment variable
3. `host-bridge.toml` in the working directory

Start from the default template: `host-bridge.toml`.

### Run (use the compiled binary)

```bash
./target/release/host-bridge-mcp --config host-bridge.toml
```

The server listens on the configured address (default: `0.0.0.0:8787`).

### Connect

- MCP endpoint: `http://127.0.0.1:8787/mcp` (transport: `streamable-http`)
- Health check: `http://127.0.0.1:8787/health`

## Usage

Add to your MCP client:

```json
{
    "type": "streamable-http",
    "url": "http://localhost:8787/mcp"
}
```

### Tool: execute_command

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

### Stream output (SSE)

Subscribe to sse to receive events: `status`, `output`, `exit`, `error`.

```bash
curl -N "http://127.0.0.1:8787/executions/<executionId>/stream"
```

### Confirmation behavior

Policies can require confirmation.

> [!IMPORTANT]
> Interactive confirmation happens in the server console and requires a TTY.
> For automation, prefer explicit allow/deny policies.

## Configuration

- Default template: `host-bridge.toml`

Common settings:

- `server.bind_address` and `server.public_base_url`
- `execution.default_policy` and `execution.command_policies`
- `execution.path_mappings` (prefix-based path rewriting)
- `execution.enable_builtin_wsl_mapping` (optional `/mnt/<drive>/...` to `C:\...` mapping)

## Security

Recommended baseline for a safe setup:

- Bind to `127.0.0.1` (or a private interface) unless you fully trust the network.
- Keep `default_policy = "confirm"` and allow-list only what you need.
- Deny high-risk subcommands (`publish`, `deploy`, etc.).
- Run as a non-root user and in a restricted working directory.

## CLI options

- `-c, --config <PATH>`: set configuration file path
- `-h, --help`: show help
- `-V, --version`: show version

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
