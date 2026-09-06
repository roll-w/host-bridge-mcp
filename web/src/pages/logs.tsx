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

import {useState} from "react";
import {Eraser, Pause, Play, Radio, RefreshCw} from "lucide-react";
import {type Locale, type MessageKey} from "@/i18n";
import type {RuntimeLog} from "@/types";
import {RuntimeLogWindow, useRuntimeLogStream,} from "@/components/runtime-log-feed";
import {PageHeading} from "@/components/layout";
import {Button} from "@/components/ui/button";
import {cn} from "@/lib/utils";

export function LogsPage({
                             t,
                             locale,
                         }: {
    t: (key: MessageKey) => string;
    locale: Locale;
}) {
    const {entries, loading, clear, refresh} = useRuntimeLogStream(500);
    const [filter, setFilter] = useState<"all" | RuntimeLog["level"]>("all");
    const [paused, setPaused] = useState(false);
    const [autoFollow, setAutoFollow] = useState(true);
    const levelLabel: Record<"all" | RuntimeLog["level"], MessageKey> = {
        all: "filterAll",
        info: "filterInfo",
        warn: "filterWarn",
        error: "filterError",
    };
    const filtered =
        filter === "all"
            ? entries
            : entries.filter((entry) => entry.level === filter);

    return (
        <div className="space-y-8">
            <PageHeading
                eyebrow={t("operatorWorkspace")}
                title={t("runtimeLogs")}
                description={t("runtimeLogsHint")}
                action={<Radio className="size-4 text-primary"/>}
            />
            <div className="flex flex-wrap items-center justify-between gap-3">
                <div className="flex flex-wrap items-center gap-1 rounded-md bg-muted/60 p-1">
                    {(["all", "info", "warn", "error"] as const).map((level) => (
                        <Button
                            key={level}
                            variant={filter === level ? "default" : "ghost"}
                            size="sm"
                            onClick={() => setFilter(level)}
                            className={cn(filter === level && "bg-primary")}
                        >
                            {t(levelLabel[level])}
                        </Button>
                    ))}
                </div>
                <div className="flex items-center gap-1">
                    <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => setAutoFollow((value) => !value)}
                    >
                        {autoFollow ? t("autoFollowOn") : t("autoFollowOff")}
                    </Button>
                    <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => setPaused((value) => !value)}
                    >
                        {paused ? (
                            <Play className="size-3.5"/>
                        ) : (
                            <Pause className="size-3.5"/>
                        )}
                        {paused ? t("resume") : t("pause")}
                    </Button>
                    <Button variant="ghost" size="sm" onClick={clear}>
                        <Eraser className="size-3.5"/>
                        {t("clear")}
                    </Button>
                    <Button
                        variant="ghost"
                        size="icon-sm"
                        onClick={refresh}
                        aria-label={t("refresh")}
                        title={t("refresh")}
                    >
                        <RefreshCw className="size-3.5"/>
                    </Button>
                </div>
            </div>
            {loading && entries.length === 0 ? (
                <div
                    className="rounded-lg border border-border bg-muted/30 p-8 text-center font-mono text-sm text-muted-foreground">
                    {t("loading")}
                </div>
            ) : (
                <RuntimeLogWindow
                    entries={filtered}
                    locale={locale}
                    t={t}
                    autoFollow={autoFollow}
                    paused={paused}
                />
            )}
        </div>
    );
}
