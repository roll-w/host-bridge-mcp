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

import {useEffect, useRef, useState} from "react";
import {CheckCircle2, CircleAlert, Info, Radio, RotateCcw,} from "lucide-react";
import {type Locale, type MessageKey} from "@/i18n";
import {apiRequest, parseEvent} from "@/api";
import {formatTime} from "@/utils";
import type {RuntimeLog, RuntimeLogPage} from "@/types";
import {EmptyState} from "@/components/layout";
import {Button} from "@/components/ui/button";
import {Card, CardContent, CardDescription, CardHeader, CardTitle,} from "@/components/ui/card";
import {ScrollArea} from "@/components/ui/scroll-area";
import {cn} from "@/lib/utils";

export function useRuntimeLogStream(maxEntries = 500) {
    const [entries, setEntries] = useState<RuntimeLog[]>([]);
    const [loading, setLoading] = useState(true);
    const [revision, setRevision] = useState(0);

    useEffect(() => {
        let active = true;
        setLoading(true);
        const source = new EventSource("/api/v1/logs/stream");
        apiRequest<RuntimeLogPage>(`/logs?offset=0&limit=${maxEntries}`)
            .then((page) => {
                if (active) setEntries(page.entries);
            })
            .catch(() => undefined)
            .finally(() => {
                if (active) setLoading(false);
            });

        source.addEventListener("snapshot", (event) => {
            const data = parseEvent<{ entries: RuntimeLog[] }>(event);
            if (active && data) {
                setEntries(data.entries.slice(-maxEntries));
            }
        });
        source.addEventListener("log", (event) => {
            const entry = parseEvent<RuntimeLog>(event);
            if (active && entry) {
                setEntries((current) => [...current, entry].slice(-maxEntries));
            }
        });
        return () => {
            active = false;
            source.close();
        };
    }, [maxEntries, revision]);

    return {
        entries,
        loading,
        clear: () => setEntries([]),
        refresh: () => setRevision((value) => value + 1),
    };
}

export function RuntimeLogPreview({
                                      locale,
                                      t,
                                      onViewAll,
                                  }: {
    locale: Locale;
    t: (key: MessageKey) => string;
    onViewAll: () => void;
}) {
    const {entries, loading} = useRuntimeLogStream(100);
    const latest = entries.slice(-6).reverse();
    return (
        <Card className="gap-0 overflow-hidden py-0">
            <CardHeader className="flex flex-row items-start justify-between gap-4 bg-muted/30 px-5 py-4 sm:px-6">
                <div className="space-y-1">
                    <CardTitle className="flex items-center gap-2 text-sm">
                        <Radio className="size-3.5"/>
                        {t("runtimeLogs")}
                    </CardTitle>
                    <CardDescription>{t("runtimeLogsHint")}</CardDescription>
                </div>
                <Button variant="ghost" size="sm" onClick={onViewAll}>
                    {t("viewAll")}
                </Button>
            </CardHeader>
            <CardContent className="px-0 py-0">
                {loading ? (
                    <div className="px-5 py-8 text-sm text-muted-foreground">
                        {t("loading")}
                    </div>
                ) : latest.length === 0 ? (
                    <EmptyState className="m-4">{t("noLogs")}</EmptyState>
                ) : (
                    <div className="space-y-1 p-3 font-mono text-xs">
                        {latest.map((entry, index) => (
                            <LogLine
                                key={`${entry.timestamp}-${index}`}
                                entry={entry}
                                locale={locale}
                            />
                        ))}
                    </div>
                )}
            </CardContent>
        </Card>
    );
}

export function LogLine({
                            entry,
                            locale,
                            dense = false,
                        }: {
    entry: RuntimeLog;
    locale: Locale;
    dense?: boolean;
}) {
    const Icon =
        entry.level === "error"
            ? CircleAlert
            : entry.level === "warn"
                ? RotateCcw
                : entry.level === "info"
                    ? Info
                    : CheckCircle2;
    return (
        <div
            className={cn(
                "grid grid-cols-[8.5rem_6rem_minmax(0,1fr)] items-start gap-x-3 rounded-md px-3 transition-colors hover:bg-muted/60 sm:grid-cols-[11.5rem_6.5rem_minmax(0,1fr)]",
                dense ? "py-1.5" : "py-2",
            )}
        >
      <span
          className="flex min-w-0 min-h-5 items-center truncate whitespace-nowrap text-[11px] leading-5 text-muted-foreground">
        {formatTime(entry.timestamp, locale)}
      </span>
            <span
                className={cn(
                    "flex min-h-5 items-center gap-1 whitespace-nowrap text-[10px] font-semibold leading-5 uppercase tracking-[0.12em]",
                    entry.level === "error"
                        ? "text-destructive"
                        : entry.level === "warn"
                            ? "text-amber-700"
                            : "text-primary/80",
                )}
            >
        <Icon className="size-3"/>
                {entry.level}
      </span>
            <span
                className={cn(
                    "min-w-0 whitespace-pre-wrap break-words leading-5 text-foreground/80",
                )}
            >
        {entry.message}
      </span>
        </div>
    );
}

export function RuntimeLogWindow({
                                     entries,
                                     locale,
                                     t,
                                     autoFollow,
                                     paused,
                                 }: {
    entries: RuntimeLog[];
    locale: Locale;
    t: (key: MessageKey) => string;
    autoFollow: boolean;
    paused: boolean;
}) {
    const endRef = useRef<HTMLDivElement>(null);
    useEffect(() => {
        if (autoFollow && !paused)
            endRef.current?.scrollIntoView({behavior: "smooth"});
    }, [entries.length, autoFollow, paused]);
    return (
        <Card className="gap-0 overflow-hidden py-0">
            <CardHeader
                className="flex flex-row items-center justify-between gap-3 bg-muted/30 px-4 py-3 text-xs sm:px-5">
        <span className="font-mono text-muted-foreground">
          {paused
              ? t("paused")
              : autoFollow
                  ? t("autoFollowOn")
                  : t("autoFollowOff")}
        </span>
            </CardHeader>
            <CardContent className="px-0 py-0">
                <ScrollArea className="h-[calc(100vh-22rem)] min-h-[420px] max-h-[620px] font-mono text-xs">
                    <div className="p-3">
                        {entries.length === 0 ? (
                            <div className="grid min-h-[360px] place-items-center text-muted-foreground">
                                {t("noLogs")}
                            </div>
                        ) : (
                            entries.map((entry, index) => (
                                <LogLine
                                    key={`${entry.timestamp}-${index}`}
                                    entry={entry}
                                    locale={locale}
                                    dense
                                />
                            ))
                        )}
                        <div ref={endRef}/>
                    </div>
                </ScrollArea>
            </CardContent>
        </Card>
    );
}
