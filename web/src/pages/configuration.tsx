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

import {useEffect, useState} from "react";
import {KeyRound, Plus, RefreshCw, Save, Trash2} from "lucide-react";
import {apiRequest} from "@/api";
import {type MessageKey} from "@/i18n";
import type {
    AppConfig,
    CommandPolicyConfig,
    CommandRuleConfig,
    ConfigSnapshot,
    ExecutionServerConfig,
    HostServerConfig,
    PolicyAction,
    SshAuthType,
    SshServerConfig,
    TargetPlatform,
    VisualConfigPatch,
} from "@/types";
import {ConfigGroup, InlineError, PageHeading, StatusPill,} from "@/components/layout";
import {Field, NumberField, SelectField, ToggleField} from "@/components/form";
import {ConfirmDialog} from "@/components/confirm-dialog";
import {Button} from "@/components/ui/button";
import {Label} from "@/components/ui/label";
import {MultiSelect} from "@/components/ui/multi-select";
import {Textarea} from "@/components/ui/textarea";
import {Tabs, TabsContent, TabsList, TabsTrigger} from "@/components/ui/tabs";

type VisualTab = "general" | "policies" | "servers";
type PasswordFileState = {
    password: string;
    writing: boolean;
    written: boolean;
    error: string | null;
};

const actionValues: PolicyAction[] = ["allow", "confirm", "deny"];
const platformValues: TargetPlatform[] = ["auto", "windows", "linux", "macos"];
const sshPlatformValues: Exclude<TargetPlatform, "auto">[] = [
    "windows",
    "linux",
    "macos",
];
const authValues: SshAuthType[] = [
    "agent",
    "identity-file",
    "password-env",
    "password-file",
];

export function ConfigurationPage({t}: { t: (key: MessageKey) => string }) {
    const [snapshot, setSnapshot] = useState<ConfigSnapshot | null>(null);
    const [draft, setDraft] = useState<AppConfig | null>(null);
    const [raw, setRaw] = useState("");
    const [tab, setTab] = useState<VisualTab>("general");
    const [rawMode, setRawMode] = useState(false);
    const [notice, setNotice] = useState<string | null>(null);
    const [error, setError] = useState<string | null>(null);
    const [saving, setSaving] = useState(false);
    const [confirmAction, setConfirmAction] = useState<(() => void) | null>(null);
    const [passwordFiles, setPasswordFiles] = useState<
        Record<number, PasswordFileState>
    >({});

    const load = () =>
        apiRequest<ConfigSnapshot>("/config")
            .then((value) => {
                setSnapshot(value);
                setDraft(cloneConfig(value.config));
                setRaw(value.raw);
                setError(null);
            })
            .catch((reason: unknown) =>
                setError(reason instanceof Error ? reason.message : t("loadFailed")),
            );
    useEffect(() => {
        load();
    }, []);

    const visualDirty = Boolean(
        snapshot &&
        draft &&
        JSON.stringify(draft) !== JSON.stringify(snapshot.config),
    );
    const rawDirty = Boolean(snapshot && raw !== snapshot.raw);
    const dirty = rawMode ? rawDirty : visualDirty;
    const updateDraft = (mutator: (next: AppConfig) => void) =>
        setDraft((current) => {
            if (!current) return current;
            const next = cloneConfig(current);
            mutator(next);
            return next;
        });

    const discard = () => {
        if (!snapshot) return;
        setDraft(cloneConfig(snapshot.config));
        setRaw(snapshot.raw);
        setNotice(null);
    };
    const refresh = () => {
        if (dirty) {
            setConfirmAction(() => () => {
                discard();
                void load();
            });
            return;
        }
        void load();
    };
    const saveVisual = async () => {
        if (!draft) return;
        setSaving(true);
        setNotice(null);
        setError(null);
        const patch: VisualConfigPatch = {
            dataDir: draft["data-dir"] ?? "",
            tui: draft.tui,
            web: draft.web,
            serverAddress: draft.server.address,
            apiKeyEnv: draft.server.access["api-key-env"] ?? "",
            logRetentionDays: draft.logging["retention-days"],
            defaultAction: draft.execution["default-action"],
            defaultWorkingDirectory:
                draft.execution["default-working-directory"] ?? "",
            defaultServer: draft.execution["default-server"],
            targetPlatform: draft.execution["target-platform"],
            defaultTimeoutMs: draft.execution["default-timeout-ms"],
            maxTimeoutMs: draft.execution["max-timeout-ms"],
            historyRetentionDays: draft.history["retention-days"],
            historyMaxRecords: draft.history["max-records"],
            commands: draft.execution.commands,
            servers: draft.execution.servers,
        };
        try {
            const value = await apiRequest<ConfigSnapshot>("/config/visual", {
                method: "PUT",
                body: JSON.stringify(patch),
            });
            setSnapshot(value);
            setDraft(cloneConfig(value.config));
            setRaw(value.raw);
            setNotice(t("saved"));
        } catch (reason) {
            setError(reason instanceof Error ? reason.message : t("loadFailed"));
        } finally {
            setSaving(false);
        }
    };
    const saveRaw = async () => {
        setSaving(true);
        setNotice(null);
        setError(null);
        try {
            const value = await apiRequest<ConfigSnapshot>("/config/raw", {
                method: "PUT",
                body: JSON.stringify({raw}),
            });
            setSnapshot(value);
            setDraft(cloneConfig(value.config));
            setRaw(value.raw);
            setNotice(t("saved"));
        } catch (reason) {
            setError(reason instanceof Error ? reason.message : t("loadFailed"));
        } finally {
            setSaving(false);
        }
    };

    const writePasswordFile = async (
        index: number,
        path: string,
        serverName: string,
    ) => {
        const state = passwordFiles[index];
        if (!state?.password) return;
        setPasswordFiles((current) => ({
            ...current,
            [index]: {...state, writing: true, error: null},
        }));
        try {
            const response = await apiRequest<{ path: string }>(
                "/config/ssh-password-file",
                {
                    method: "POST",
                    body: JSON.stringify({
                        path: path.trim() || null,
                        serverName,
                        password: state.password,
                    }),
                },
            );
            if (!path.trim()) {
                updateDraft((next) => {
                    const target = next.execution.servers[index];
                    if (target?.transport === "ssh") target.auth.ref = response.path;
                });
            }
            setPasswordFiles((current) => ({
                ...current,
                [index]: {
                    password: "",
                    writing: false,
                    written: true,
                    error: null,
                },
            }));
            setNotice(t("passwordFileWritten"));
        } catch (reason) {
            setPasswordFiles((current) => ({
                ...current,
                [index]: {
                    ...state,
                    writing: false,
                    written: false,
                    error: reason instanceof Error ? reason.message : t("loadFailed"),
                },
            }));
        }
    };

    if (error && !snapshot)
        return (
            <div className="space-y-4">
                <InlineError message={error}/>
                <Button variant="outline" onClick={load}>
                    {t("retry")}
                </Button>
            </div>
        );
    if (!snapshot || !draft)
        return <div className="text-sm text-muted-foreground">{t("loading")}</div>;

    const applyTab = (value: string) => {
        discard();
        setRawMode(value === "raw");
        if (value !== "raw") setTab(value as VisualTab);
    };
    const changeTab = (value: string) => {
        if (dirty) {
            setConfirmAction(() => () => applyTab(value));
            return;
        }
        applyTab(value);
    };
    return (
        <div className="space-y-8">
            <PageHeading
                eyebrow={t("operatorWorkspace")}
                title={t("configuration")}
                description={`${t("configPath")}: ${snapshot.path}`}
                action={
                    <div className="flex items-center gap-2">
                        {dirty && (
                            <StatusPill tone="warn">{t("unsavedChanges")}</StatusPill>
                        )}
                        <Button variant="outline" size="sm" onClick={refresh}>
                            <RefreshCw className="size-3.5"/>
                            {t("refresh")}
                        </Button>
                    </div>
                }
            />
            {notice && (
                <div className="rounded-lg bg-emerald-50 px-4 py-3 text-sm text-emerald-800">
                    {notice}
                </div>
            )}
            {error && <InlineError message={error}/>}
            <Tabs
                value={rawMode ? "raw" : tab}
                onValueChange={changeTab}
                className="min-w-0 gap-6"
            >
                <div className="min-w-0 overflow-x-auto pb-1">
                    <TabsList
                        variant="line"
                        className="w-max min-w-full justify-start gap-1 rounded-none bg-transparent p-0"
                    >
                        <TabsTrigger value="general" className="shrink-0 px-3 py-2">
                            {t("generalSettings")}
                        </TabsTrigger>
                        <TabsTrigger value="policies" className="shrink-0 px-3 py-2">
                            {t("commandPolicies")}
                        </TabsTrigger>
                        <TabsTrigger value="servers" className="shrink-0 px-3 py-2">
                            {t("executionTargets")}
                        </TabsTrigger>
                        <TabsTrigger value="raw" className="shrink-0 px-3 py-2">
                            {t("advancedYaml")}
                        </TabsTrigger>
                    </TabsList>
                </div>
                <TabsContent value="general">
                    <GeneralConfig draft={draft} updateDraft={updateDraft} t={t}/>
                </TabsContent>
                <TabsContent value="policies">
                    <PoliciesEditor
                        commands={draft.execution.commands}
                        targetNames={[
                            "host",
                            ...draft.execution.servers.map((server) => server.name),
                        ]}
                        updateDraft={updateDraft}
                        t={t}
                    />
                </TabsContent>
                <TabsContent value="servers">
                    <ServersEditor
                        servers={draft.execution.servers}
                        passwordFiles={passwordFiles}
                        updateDraft={updateDraft}
                        onPasswordChange={(index, password) =>
                            setPasswordFiles((current) => ({
                                ...current,
                                [index]: {
                                    ...(current[index] ?? {
                                        writing: false,
                                        written: false,
                                        error: null,
                                    }),
                                    password,
                                    written: false,
                                    error: null,
                                },
                            }))
                        }
                        onWritePasswordFile={writePasswordFile}
                        t={t}
                    />
                </TabsContent>
                <TabsContent value="raw">
                    <div className="space-y-4">
                        <p className="text-sm leading-6 text-muted-foreground">
                            {t("rawYamlHint")}
                        </p>
                        <Textarea
                            value={raw}
                            onChange={(event) => setRaw(event.target.value)}
                            spellCheck={false}
                            className="min-h-[650px] rounded-lg bg-muted/40 p-5 font-mono text-xs leading-5 text-foreground placeholder:text-muted-foreground"
                        />
                    </div>
                </TabsContent>
            </Tabs>
            <div
                className="sticky bottom-4 z-20 flex flex-wrap justify-end gap-3 rounded-lg border border-border bg-background/95 px-4 py-3 backdrop-blur">
                <div className="flex gap-2">
                    <Button
                        variant="ghost"
                        size="sm"
                        disabled={!dirty || saving}
                        onClick={discard}
                    >
                        {t("discard")}
                    </Button>
                    <Button
                        variant="default"
                        size="sm"
                        disabled={!dirty || saving}
                        onClick={rawMode ? saveRaw : saveVisual}
                    >
                        <Save className="size-3.5"/>
                        {saving ? t("saving") : t("save")}
                    </Button>
                </div>
            </div>
            <ConfirmDialog
                open={Boolean(confirmAction)}
                title={t("discardConfirmTitle")}
                description={t("discardConfirmDescription")}
                confirmLabel={t("discardConfirm")}
                cancelLabel={t("cancel")}
                onCancel={() => setConfirmAction(null)}
                onConfirm={() => {
                    const action = confirmAction;
                    setConfirmAction(null);
                    action?.();
                }}
            />
        </div>
    );
}

function GeneralConfig({
                           draft,
                           updateDraft,
                           t,
                       }: {
    draft: AppConfig;
    updateDraft: (mutator: (next: AppConfig) => void) => void;
    t: (key: MessageKey) => string;
}) {
    const actionOptions = actionValues.map((value) => ({
        value,
        label: t(actionLabel(value)),
    }));
    const platformOptions = platformValues.map((value) => ({
        value,
        label: t(platformLabel(value)),
    }));
    const serverOptions = [
        "host",
        ...draft.execution.servers.map((server) => server.name),
    ]
        .filter((value, index, values) => value && values.indexOf(value) === index)
        .map((value) => ({value, label: value}));
    return (
        <div className="grid gap-6 xl:grid-cols-2">
            <ConfigGroup
                title={t("serverSection")}
                description={t("serverSectionHint")}
            >
                <div className="grid gap-x-6 gap-y-5 sm:grid-cols-2">
                    <Field
                        label={t("dataDirectory")}
                        value={draft["data-dir"] ?? ""}
                        onChange={(value) =>
                            updateDraft((next) => {
                                next["data-dir"] = value || null;
                            })
                        }
                        mono
                        placeholder={t("dataDirectoryDefault")}
                        description={t("dataDirectoryHint")}
                    />
                    <Field
                        label={t("serverAddress")}
                        value={draft.server.address}
                        onChange={(value) =>
                            updateDraft((next) => {
                                next.server.address = value;
                            })
                        }
                        mono
                    />
                    <Field
                        label={t("apiKeyEnv")}
                        value={draft.server.access["api-key-env"] ?? ""}
                        onChange={(value) =>
                            updateDraft((next) => {
                                next.server.access["api-key-env"] = value || null;
                            })
                        }
                        mono
                        description={t("apiKeyEnvHint")}
                    />
                </div>
            </ConfigGroup>
            <ConfigGroup
                title={t("interfaceSection")}
                description={t("interfaceSectionHint")}
            >
                <div className="grid gap-x-6 gap-y-5 sm:grid-cols-2">
                    <ToggleField
                        label={t("tuiEnabled")}
                        value={draft.tui}
                        onChange={(value) =>
                            updateDraft((next) => {
                                next.tui = value;
                            })
                        }
                        description={t("restartRequiredHint")}
                    />
                    <ToggleField
                        label={t("webEnabled")}
                        value={draft.web}
                        onChange={(value) =>
                            updateDraft((next) => {
                                next.web = value;
                            })
                        }
                        description={t("webEnabledHint")}
                    />
                </div>
            </ConfigGroup>
            <ConfigGroup
                title={t("loggingSection")}
                description={t("loggingSectionHint")}
            >
                <div className="grid gap-x-6 gap-y-5 sm:grid-cols-2">
                    <NumberField
                        label={t("logRetentionDays")}
                        value={draft.logging["retention-days"]}
                        min={0}
                        onChange={(value) =>
                            updateDraft((next) => {
                                next.logging["retention-days"] = value;
                            })
                        }
                        description={t("logRetentionDaysHint")}
                    />
                </div>
            </ConfigGroup>
            <ConfigGroup
                title={t("executionSection")}
                description={t("executionSectionHint")}
            >
                <div className="grid gap-x-6 gap-y-5 sm:grid-cols-2">
                    <SelectField
                        label={t("defaultAction")}
                        value={draft.execution["default-action"]}
                        options={actionOptions}
                        onChange={(value) =>
                            updateDraft((next) => {
                                next.execution["default-action"] = value as PolicyAction;
                            })
                        }
                    />
                    <SelectField
                        label={t("defaultServer")}
                        value={draft.execution["default-server"]}
                        options={serverOptions}
                        onChange={(value) =>
                            updateDraft((next) => {
                                next.execution["default-server"] = value;
                            })
                        }
                    />
                    <SelectField
                        label={t("targetPlatform")}
                        value={draft.execution["target-platform"]}
                        options={platformOptions}
                        onChange={(value) =>
                            updateDraft((next) => {
                                next.execution["target-platform"] = value as TargetPlatform;
                            })
                        }
                    />
                    <Field
                        label={t("defaultWorkingDirectory")}
                        value={draft.execution["default-working-directory"] ?? ""}
                        onChange={(value) =>
                            updateDraft((next) => {
                                next.execution["default-working-directory"] = value || null;
                            })
                        }
                        mono
                    />
                    <NumberField
                        label={t("defaultTimeout")}
                        value={draft.execution["default-timeout-ms"]}
                        onChange={(value) =>
                            updateDraft((next) => {
                                next.execution["default-timeout-ms"] = value;
                            })
                        }
                    />
                    <NumberField
                        label={t("maxTimeout")}
                        value={draft.execution["max-timeout-ms"]}
                        onChange={(value) =>
                            updateDraft((next) => {
                                next.execution["max-timeout-ms"] = value;
                            })
                        }
                    />
                </div>
            </ConfigGroup>
            <ConfigGroup title={t("history")} description={t("historySectionHint")}>
                <div className="grid gap-x-6 gap-y-5 sm:grid-cols-2">
                    <NumberField
                        label={t("retentionDays")}
                        value={draft.history["retention-days"]}
                        onChange={(value) =>
                            updateDraft((next) => {
                                next.history["retention-days"] = value;
                            })
                        }
                    />
                    <NumberField
                        label={t("maxRecords")}
                        value={draft.history["max-records"]}
                        onChange={(value) =>
                            updateDraft((next) => {
                                next.history["max-records"] = value;
                            })
                        }
                    />
                </div>
            </ConfigGroup>
        </div>
    );
}

function PoliciesEditor({
                            commands,
                            targetNames,
                            updateDraft,
                            t,
                        }: {
    commands: CommandPolicyConfig[];
    targetNames: string[];
    updateDraft: (mutator: (next: AppConfig) => void) => void;
    t: (key: MessageKey) => string;
}) {
    const options = actionValues.map((value) => ({
        value,
        label: t(actionLabel(value)),
    }));
    const targetOptions = targetNames
        .filter((value, index, values) => value && values.indexOf(value) === index)
        .map((value) => ({value, label: value}));
    return (
        <div className="space-y-5">
            <div className="flex flex-wrap items-end justify-between gap-3">
                <div>
                    <h2 className="font-heading text-lg font-semibold">
                        {t("commandPolicies")}
                    </h2>
                    <p className="mt-1 text-sm leading-6 text-muted-foreground">
                        {t("commandPoliciesHint")}
                    </p>
                    <p className="mt-2 max-w-2xl text-xs leading-5 text-muted-foreground">
                        {t("wildcardHint")}
                    </p>
                </div>
                <Button
                    variant="outline"
                    size="sm"
                    onClick={() =>
                        updateDraft((next) => {
                            next.execution.commands.push(newCommand());
                        })
                    }
                >
                    <Plus className="size-3.5"/>
                    {t("addCommandPolicy")}
                </Button>
            </div>
            {commands.length === 0 ? (
                <div className="rounded-2xl bg-card/70 p-8 text-center text-sm text-muted-foreground">
                    {t("noCommandPolicies")}
                </div>
            ) : (
                commands.map((command, index) => (
                    <section
                        key={index}
                        className="rounded-lg border border-border bg-card p-5 sm:p-6"
                    >
                        <div className="mb-5 flex items-start justify-between gap-3">
                            <div>
                                <div className="text-[11px] font-semibold uppercase tracking-[0.16em] text-primary">
                                    {t("policy")} {index + 1}
                                </div>
                                <p className="mt-1 text-xs text-muted-foreground">
                                    {t("policyEditorHint")}
                                </p>
                            </div>
                            <Button
                                variant="ghost"
                                size="icon-sm"
                                onClick={() =>
                                    updateDraft((next) => {
                                        next.execution.commands.splice(index, 1);
                                    })
                                }
                                aria-label={t("remove")}
                                title={t("remove")}
                            >
                                <Trash2 className="size-3.5 text-destructive"/>
                            </Button>
                        </div>
                        <div className="grid gap-5 lg:grid-cols-[1.4fr_1fr_1.4fr]">
                            <Field
                                label={t("commandName")}
                                value={command.command}
                                onChange={(value) =>
                                    updateDraft((next) => {
                                        next.execution.commands[index].command = value;
                                    })
                                }
                                mono
                                placeholder="*"
                            />
                            <SelectField
                                label={t("policyAction")}
                                value={command.action}
                                options={options}
                                onChange={(value) =>
                                    updateDraft((next) => {
                                        next.execution.commands[index].action =
                                            value as PolicyAction;
                                    })
                                }
                            />
                            <Field
                                label={t("policyWorkingDirectory")}
                                value={command["default-working-directory"] ?? ""}
                                onChange={(value) =>
                                    updateDraft((next) => {
                                        next.execution.commands[index][
                                            "default-working-directory"
                                            ] = value || null;
                                    })
                                }
                                mono
                            />
                        </div>
                        <div className="mt-5 max-w-xl space-y-2">
                            <Label>{t("policyTargets")}</Label>
                            <MultiSelect
                                options={targetOptions}
                                value={command.targets}
                                onChange={(value) =>
                                    updateDraft((next) => {
                                        next.execution.commands[index].targets = value;
                                    })
                                }
                                placeholder={t("policyTargetsPlaceholder")}
                                allLabel={t("allTargets")}
                            />
                        </div>
                        <div className="mt-6 space-y-3">
                            <div className="flex items-center justify-between gap-3">
                                <div>
                                    <h3 className="text-sm font-semibold">{t("rules")}</h3>
                                    <p className="mt-1 text-xs text-muted-foreground">
                                        {t("rulesHint")}
                                    </p>
                                </div>
                                <Button
                                    variant="ghost"
                                    size="sm"
                                    onClick={() =>
                                        updateDraft((next) => {
                                            next.execution.commands[index].rules.push(newRule());
                                        })
                                    }
                                >
                                    <Plus className="size-3.5"/>
                                    {t("addRule")}
                                </Button>
                            </div>
                            {command.rules.length === 0 ? (
                                <p className="rounded-lg bg-muted/45 px-3 py-3 text-xs text-muted-foreground">
                                    {t("noRules")}
                                </p>
                            ) : (
                                command.rules.map((rule, ruleIndex) => (
                                    <RuleEditor
                                        key={ruleIndex}
                                        rule={rule}
                                        index={ruleIndex}
                                        commandIndex={index}
                                        updateDraft={updateDraft}
                                        options={options}
                                        t={t}
                                    />
                                ))
                            )}
                        </div>
                    </section>
                ))
            )}
        </div>
    );
}

function RuleEditor({
                        rule,
                        index,
                        commandIndex,
                        updateDraft,
                        options,
                        t,
                    }: {
    rule: CommandRuleConfig;
    index: number;
    commandIndex: number;
    updateDraft: (mutator: (next: AppConfig) => void) => void;
    options: Array<{ value: string; label: string }>;
    t: (key: MessageKey) => string;
}) {
    return (
        <div className="rounded-md bg-muted/45 p-4">
            <div className="mb-3 flex items-center justify-between">
        <span className="text-xs font-semibold text-muted-foreground">
          {t("rule")} {index + 1}
        </span>
                <Button
                    variant="ghost"
                    size="icon-sm"
                    onClick={() =>
                        updateDraft((next) => {
                            next.execution.commands[commandIndex].rules.splice(index, 1);
                        })
                    }
                    aria-label={t("remove")}
                    title={t("remove")}
                >
                    <Trash2 className="size-3.5 text-destructive"/>
                </Button>
            </div>
            <div className="grid gap-5 lg:grid-cols-[1.5fr_1fr_1.3fr]">
                <div className="space-y-2">
                    <Label>{t("argsPrefix")}</Label>
                    <Textarea
                        value={rule["args-prefix"].join("\n")}
                        onChange={(event) =>
                            updateDraft((next) => {
                                next.execution.commands[commandIndex].rules[index][
                                    "args-prefix"
                                    ] = splitTokens(event.target.value);
                            })
                        }
                        className="min-h-20 font-mono text-xs"
                        placeholder={t("argsPrefixPlaceholder")}
                    />
                </div>
                <SelectField
                    label={t("policyAction")}
                    value={rule.action}
                    options={options}
                    onChange={(value) =>
                        updateDraft((next) => {
                            next.execution.commands[commandIndex].rules[index].action =
                                value as PolicyAction;
                        })
                    }
                />
                <Field
                    label={t("policyWorkingDirectory")}
                    value={rule["default-working-directory"] ?? ""}
                    onChange={(value) =>
                        updateDraft((next) => {
                            next.execution.commands[commandIndex].rules[index][
                                "default-working-directory"
                                ] = value || null;
                        })
                    }
                    mono
                />
            </div>
        </div>
    );
}

function ServersEditor({
                           servers,
                           passwordFiles,
                           updateDraft,
                           onPasswordChange,
                           onWritePasswordFile,
                           t,
                       }: {
    servers: ExecutionServerConfig[];
    passwordFiles: Record<number, PasswordFileState>;
    updateDraft: (mutator: (next: AppConfig) => void) => void;
    onPasswordChange: (index: number, password: string) => void;
    onWritePasswordFile: (
        index: number,
        path: string,
        serverName: string,
    ) => void;
    t: (key: MessageKey) => string;
}) {
    return (
        <div className="space-y-5">
            <div className="flex flex-wrap items-end justify-between gap-3">
                <div>
                    <h2 className="font-heading text-lg font-semibold">
                        {t("executionTargets")}
                    </h2>
                    <p className="mt-1 text-sm leading-6 text-muted-foreground">
                        {t("executionTargetsHint")}
                    </p>
                </div>
                <div className="flex gap-2">
                    <Button
                        variant="ghost"
                        size="sm"
                        onClick={() =>
                            updateDraft((next) => {
                                next.execution.servers.push(newHost());
                            })
                        }
                    >
                        <Plus className="size-3.5"/>
                        {t("addHost")}
                    </Button>
                    <Button
                        variant="outline"
                        size="sm"
                        onClick={() =>
                            updateDraft((next) => {
                                next.execution.servers.push(newSsh());
                            })
                        }
                    >
                        <Plus className="size-3.5"/>
                        {t("addSsh")}
                    </Button>
                </div>
            </div>
            {servers.length === 0 ? (
                <div className="rounded-2xl bg-card/70 p-8 text-center text-sm text-muted-foreground">
                    {t("noExecutionTargets")}
                </div>
            ) : (
                servers.map((server, index) => (
                    <ServerEditor
                        key={index}
                        server={server}
                        index={index}
                        passwordFile={passwordFiles[index]}
                        updateDraft={updateDraft}
                        onPasswordChange={onPasswordChange}
                        onWritePasswordFile={onWritePasswordFile}
                        t={t}
                    />
                ))
            )}
        </div>
    );
}

function ServerEditor({
                          server,
                          index,
                          passwordFile,
                          updateDraft,
                          onPasswordChange,
                          onWritePasswordFile,
                          t,
                      }: {
    server: ExecutionServerConfig;
    index: number;
    passwordFile?: PasswordFileState;
    updateDraft: (mutator: (next: AppConfig) => void) => void;
    onPasswordChange: (index: number, password: string) => void;
    onWritePasswordFile: (
        index: number,
        path: string,
        serverName: string,
    ) => void;
    t: (key: MessageKey) => string;
}) {
    const platformOptions = (
        server.transport === "host" ? platformValues : sshPlatformValues
    ).map((value) => ({value, label: t(platformLabel(value))}));
    const authRefLabel =
        server.transport === "ssh" && server.auth.type === "password-env"
            ? t("sshPasswordEnvName")
            : server.transport === "ssh" && server.auth.type === "password-file"
                ? t("sshPasswordFilePath")
                : t("sshAuthRef");
    const authRefDescription =
        server.transport === "ssh" && server.auth.type === "password-env"
            ? t("sshPasswordEnvHint")
            : server.transport === "ssh" && server.auth.type === "password-file"
                ? t("sshPasswordFileHint")
                : undefined;
    return (
        <section className="rounded-lg border border-border bg-card p-5 sm:p-6">
            <div className="mb-5 flex items-start justify-between gap-3">
                <div>
                    <div className="text-[11px] font-semibold uppercase tracking-[0.16em] text-primary">
                        {server.transport === "ssh" ? t("sshTarget") : t("hostTarget")}{" "}
                        {index + 1}
                    </div>
                    <p className="mt-1 text-xs text-muted-foreground">
                        {server.transport === "ssh"
                            ? t("sshTargetHint")
                            : t("hostTargetHint")}
                    </p>
                </div>
                <Button
                    variant="ghost"
                    size="icon-sm"
                    onClick={() =>
                        updateDraft((next) => {
                            next.execution.servers.splice(index, 1);
                        })
                    }
                    aria-label={t("remove")}
                    title={t("remove")}
                >
                    <Trash2 className="size-3.5 text-destructive"/>
                </Button>
            </div>
            <div className="grid gap-5 sm:grid-cols-2 lg:grid-cols-3">
                <Field
                    label={t("serverName")}
                    value={server.name}
                    onChange={(value) =>
                        updateDraft((next) => {
                            next.execution.servers[index].name = value;
                        })
                    }
                    mono
                />
                <SelectField
                    label={t("targetPlatform")}
                    value={server["target-platform"]}
                    options={platformOptions}
                    onChange={(value) =>
                        updateDraft((next) => {
                            next.execution.servers[index]["target-platform"] =
                                value as TargetPlatform;
                        })
                    }
                />
                {server.transport === "ssh" && (
                    <>
                        <Field
                            label={t("sshHost")}
                            value={server.host}
                            onChange={(value) =>
                                updateDraft((next) => {
                                    const target = next.execution.servers[index];
                                    if (target.transport === "ssh") target.host = value;
                                })
                            }
                            mono
                        />
                        <NumberField
                            label={t("sshPort")}
                            value={server.port}
                            onChange={(value) =>
                                updateDraft((next) => {
                                    const target = next.execution.servers[index];
                                    if (target.transport === "ssh") target.port = value;
                                })
                            }
                        />
                        <Field
                            label={t("sshUser")}
                            value={server.user}
                            onChange={(value) =>
                                updateDraft((next) => {
                                    const target = next.execution.servers[index];
                                    if (target.transport === "ssh") target.user = value;
                                })
                            }
                            mono
                        />
                        <SelectField
                            label={t("sshAuthType")}
                            value={server.auth.type}
                            options={authValues.map((value) => ({
                                value,
                                label: t(authLabel(value)),
                            }))}
                            onChange={(value) =>
                                updateDraft((next) => {
                                    const target = next.execution.servers[index];
                                    if (target.transport === "ssh") {
                                        target.auth.type = value as SshAuthType;
                                        if (value === "agent") target.auth.ref = null;
                                    }
                                })
                            }
                        />
                        {server.auth.type !== "agent" && (
                            <Field
                                label={authRefLabel}
                                value={server.auth.ref ?? ""}
                                onChange={(value) =>
                                    updateDraft((next) => {
                                        const target = next.execution.servers[index];
                                        if (target.transport === "ssh")
                                            target.auth.ref = value || null;
                                    })
                                }
                                mono
                                placeholder={
                                    server.auth.type === "password-file"
                                        ? t("sshPasswordFileAuto")
                                        : undefined
                                }
                                description={authRefDescription}
                            />
                        )}
                        {server.auth.type === "password-file" && (
                            <div className="space-y-2">
                                <Field
                                    label={t("sshPassword")}
                                    value={passwordFile?.password ?? ""}
                                    onChange={(value) => onPasswordChange(index, value)}
                                    type="password"
                                    description={t("sshPasswordFileWriteHint")}
                                />
                                <Button
                                    variant="outline"
                                    size="sm"
                                    disabled={
                                        passwordFile?.writing === true || !passwordFile?.password
                                    }
                                    onClick={() =>
                                        onWritePasswordFile(
                                            index,
                                            server.auth.ref ?? "",
                                            server.name,
                                        )
                                    }
                                >
                                    <KeyRound className="size-3.5"/>
                                    {passwordFile?.writing
                                        ? t("writingPasswordFile")
                                        : t("writePasswordFile")}
                                </Button>
                                {passwordFile?.written && (
                                    <p className="text-xs leading-5 text-emerald-700">
                                        {t("passwordFileReady")}
                                    </p>
                                )}
                                {passwordFile?.error && (
                                    <p className="text-xs leading-5 text-destructive">
                                        {passwordFile.error}
                                    </p>
                                )}
                            </div>
                        )}
                        <Field
                            label={t("knownHostsFile")}
                            value={server["known-hosts-file"] ?? ""}
                            onChange={(value) =>
                                updateDraft((next) => {
                                    const target = next.execution.servers[index];
                                    if (target.transport === "ssh")
                                        target["known-hosts-file"] = value || null;
                                })
                            }
                            mono
                        />
                        <NumberField
                            label={t("connectionIdleTimeout")}
                            value={server["connection-idle-timeout-ms"]}
                            onChange={(value) =>
                                updateDraft((next) => {
                                    const target = next.execution.servers[index];
                                    if (target.transport === "ssh")
                                        target["connection-idle-timeout-ms"] = value;
                                })
                            }
                        />
                    </>
                )}
            </div>
        </section>
    );
}

function cloneConfig(value: AppConfig): AppConfig {
    return JSON.parse(JSON.stringify(value)) as AppConfig;
}

function splitTokens(value: string): string[] {
    return value
        .split(/\r?\n/)
        .map((token) => token.trim())
        .filter(Boolean);
}

function newCommand(): CommandPolicyConfig {
    return {
        command: "new-command",
        action: "confirm",
        targets: [],
        "default-working-directory": null,
        rules: [],
    };
}

function newRule(): CommandRuleConfig {
    return {
        "args-prefix": ["--option"],
        action: "confirm",
        "default-working-directory": null,
    };
}

function newHost(): HostServerConfig {
    return {transport: "host", name: "new-host", "target-platform": "auto"};
}

function newSsh(): SshServerConfig {
    return {
        transport: "ssh",
        name: "new-ssh",
        host: "127.0.0.1",
        port: 22,
        user: "user",
        "target-platform": "linux",
        auth: {type: "agent", ref: null},
        "known-hosts-file": null,
        "connection-idle-timeout-ms": 300000,
    };
}

function actionLabel(value: PolicyAction): MessageKey {
    return value === "allow"
        ? "policyAllow"
        : value === "deny"
            ? "policyDeny"
            : "policyConfirm";
}

function platformLabel(value: TargetPlatform): MessageKey {
    return value === "windows"
        ? "platformWindows"
        : value === "linux"
            ? "platformLinux"
            : value === "macos"
                ? "platformMacos"
                : "platformAuto";
}

function authLabel(value: SshAuthType): MessageKey {
    return value === "identity-file"
        ? "authIdentityFile"
        : value === "password-env"
            ? "authPasswordEnv"
            : value === "password-file"
                ? "authPasswordFile"
                : "authAgent";
}
