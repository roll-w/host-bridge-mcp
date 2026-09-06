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

export type Page = "overview" | "workspace" | "logs" | "config";

export interface ConfirmationRequest {
    server: string;
    platform: string;
    commandLine: string;
    executable: string;
    args: string[];
    workingDirectory: string | null;
    timeoutMs: number;
    env: Record<string, string>;
    containsShellOperator: boolean;
}

export interface PendingApproval {
    id: string;
    executionId: string;
    request: ConfirmationRequest;
    createdAt: string;
}

export type ApprovalDecision = "approve-once" | "reject";

export interface ConsoleSnapshot {
    interactive: boolean;
    pendingApprovals: PendingApproval[];
}

export interface Overview {
    defaultEnvironment: string;
    environments: { name: string; platform: string }[];
    console: ConsoleSnapshot;
    apiKeyConfigured: boolean;
}

export interface RuntimeLog {
    timestamp: string;
    level: "info" | "warn" | "error";
    message: string;
}

export interface RuntimeLogPage {
    entries: RuntimeLog[];
    offset: number;
    limit: number;
}

export interface HistoryRecord {
    executionId: string;
    commandLine: string;
    server: string;
    state: "running" | "completed" | "failed";
    startedAt: number;
    finishedAt: number | null;
    exitCode: number | null;
    success: boolean | null;
    timedOut: boolean | null;
}

export interface HistoryPage {
    records: HistoryRecord[];
    total: number;
    offset: number;
    limit: number;
}

export interface AppConfig {
    "data-dir": string | null;
    tui: boolean;
    web: boolean;
    server: { address: string; access: { "api-key-env": string | null } };
    logging: { "retention-days": number };
    execution: {
        "default-action": PolicyAction;
        commands: CommandPolicyConfig[];
        "default-working-directory": string | null;
        "default-server": string;
        servers: ExecutionServerConfig[];
        "target-platform": TargetPlatform;
        "default-timeout-ms": number;
        "max-timeout-ms": number;
    };
    history: { "retention-days": number; "max-records": number };
}

export type PolicyAction = "allow" | "confirm" | "deny";
export type TargetPlatform = "auto" | "windows" | "linux" | "macos";
export type SshAuthType =
    "agent" | "identity-file" | "password-env" | "password-file";

export interface CommandRuleConfig {
    "args-prefix": string[];
    action: PolicyAction;
    "default-working-directory": string | null;
}

export interface CommandPolicyConfig {
    command: string;
    action: PolicyAction;
    targets: string[];
    "default-working-directory": string | null;
    rules: CommandRuleConfig[];
}

export interface HostServerConfig {
    transport: "host";
    name: string;
    "target-platform": TargetPlatform;
}

export interface SshServerConfig {
    transport: "ssh";
    name: string;
    host: string;
    port: number;
    user: string;
    "target-platform": Exclude<TargetPlatform, "auto">;
    auth: { type: SshAuthType; ref: string | null };
    "known-hosts-file": string | null;
    "connection-idle-timeout-ms": number;
}

export type ExecutionServerConfig = HostServerConfig | SshServerConfig;

export interface ConfigSnapshot {
    path: string;
    raw: string;
    config: AppConfig;
}

export interface VisualConfigPatch {
    dataDir: string;
    tui: boolean;
    web: boolean;
    serverAddress: string;
    apiKeyEnv: string;
    logRetentionDays: number;
    defaultAction: PolicyAction;
    defaultWorkingDirectory: string;
    defaultServer: string;
    targetPlatform: TargetPlatform;
    defaultTimeoutMs: number;
    maxTimeoutMs: number;
    historyRetentionDays: number;
    historyMaxRecords: number;
    commands: CommandPolicyConfig[];
    servers: ExecutionServerConfig[];
}
