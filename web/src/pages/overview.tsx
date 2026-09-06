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
import {ArrowRight, CheckCircle2, RefreshCw, Server} from "lucide-react";
import {apiRequest} from "@/api";
import {type Locale, type MessageKey} from "@/i18n";
import {formatTime, stateLabel} from "@/utils";
import type {HistoryPage, HistoryRecord, Overview, Page} from "@/types";
import {ApprovalRows} from "@/components/approval";
import {RuntimeLogPreview} from "@/components/runtime-log-feed";
import {EmptyState, ErrorState, Metric, PageHeading, SectionHeading, StatusPill,} from "@/components/layout";
import {Button} from "@/components/ui/button";

export function OverviewPage({
                                 t,
                                 locale,
                                 onPageChange,
                             }: {
    t: (key: MessageKey) => string;
    locale: Locale;
    onPageChange: (page: Page) => void;
}) {
    const [overview, setOverview] = useState<Overview | null>(null);
    const [recentHistory, setRecentHistory] = useState<HistoryRecord[]>([]);
    const [error, setError] = useState<string | null>(null);
    const load = () => {
        setError(null);
        Promise.all([
            apiRequest<Overview>("/overview"),
            apiRequest<HistoryPage>("/history?offset=0&limit=5"),
        ])
            .then(([nextOverview, history]) => {
                setOverview(nextOverview);
                setRecentHistory(history.records);
            })
            .catch((reason: unknown) =>
                setError(reason instanceof Error ? reason.message : t("loadFailed")),
            );
    };
    useEffect(() => {
        load();
        const timer = window.setInterval(load, 5000);
        return () => window.clearInterval(timer);
    }, []);

    if (error) return <ErrorState message={error} onRetry={load} t={t}/>;
    if (!overview)
        return <div className="text-sm text-muted-foreground">{t("loading")}</div>;

    const pending = overview.console.pendingApprovals.length;
    return (
        <div className="space-y-9">
            <PageHeading
                eyebrow={t("operations")}
                title={t("overview")}
                description={t("overviewHint")}
                action={
                    <Button variant="outline" size="sm" onClick={load}>
                        <RefreshCw className="size-3.5"/>
                        {t("refresh")}
                    </Button>
                }
            />
            <div className="grid gap-y-4 sm:grid-cols-3 sm:gap-x-8">
                <Metric
                    label={t("defaultEnvironment")}
                    value={overview.defaultEnvironment}
                    mono
                    detail={t("activeTarget")}
                />
                <Metric
                    label={t("configuredTargets")}
                    value={overview.environments.length}
                    detail={t("executionTargets")}
                    onClick={() => onPageChange("config")}
                />
                <Metric
                    label={t("pendingApprovals")}
                    value={pending}
                    detail={pending ? t("needsAttention") : t("nothingWaiting")}
                    onClick={() => onPageChange("workspace")}
                />
            </div>

            <div className="grid gap-6 xl:grid-cols-[1.1fr_.9fr]">
                <section className="space-y-4">
                    <SectionHeading
                        title={t("environments")}
                        action={<Server className="size-4 text-primary"/>}
                    />
                    {overview.environments.length === 0 ? (
                        <EmptyState>{t("noEnvironments")}</EmptyState>
                    ) : (
                        <div className="grid gap-2 sm:grid-cols-2">
                            {overview.environments.map((environment) => (
                                <div
                                    key={environment.name}
                                    className="flex items-center gap-3 rounded-xl bg-muted/45 px-4 py-3"
                                >
                  <span className="grid size-8 place-items-center rounded-lg bg-primary/10 text-primary">
                    <Server className="size-4"/>
                  </span>
                                    <div className="min-w-0 flex-1">
                                        <div className="truncate font-mono text-sm">
                                            {environment.name}
                                        </div>
                                        <div className="mt-1 text-xs text-muted-foreground">
                                            {environment.platform}
                                        </div>
                                    </div>
                                    <CheckCircle2 className="size-4 text-emerald-600"/>
                                </div>
                            ))}
                        </div>
                    )}
                    <Button
                        variant="ghost"
                        size="sm"
                        className="mt-4 gap-1 px-0 text-primary hover:bg-transparent hover:text-primary hover:underline"
                        onClick={() => onPageChange("config")}
                    >
                        {t("manageConfiguration")}
                        <ArrowRight className="size-3"/>
                    </Button>
                </section>
                <section className="space-y-4">
                    <SectionHeading
                        title={t("pendingApprovals")}
                        action={
                            pending > 0 ? (
                                <Button
                                    variant="ghost"
                                    size="sm"
                                    onClick={() => onPageChange("workspace")}
                                >
                                    {t("viewAll")}
                                </Button>
                            ) : undefined
                        }
                    />
                    {pending === 0 ? (
                        <EmptyState className="bg-emerald-50/50 text-emerald-800">
                            <CheckCircle2 className="mx-auto mb-2 size-5"/>
                            {t("noPendingApprovals")}
                        </EmptyState>
                    ) : (
                        <ApprovalRows
                            items={overview.console.pendingApprovals.slice(0, 3)}
                            t={t}
                            locale={locale}
                            compact
                            onOpen={() => onPageChange("workspace")}
                        />
                    )}
                </section>
            </div>
            <section className="space-y-4">
                <SectionHeading
                    title={t("recentExecutions")}
                    action={
                        <Button
                            variant="ghost"
                            size="sm"
                            onClick={() => onPageChange("workspace")}
                        >
                            {t("viewAll")}
                        </Button>
                    }
                />
                {recentHistory.length === 0 ? (
                    <EmptyState>{t("noHistory")}</EmptyState>
                ) : (
                    <div className="space-y-1">
                        {recentHistory.map((record) => (
                            <Button
                                key={record.executionId}
                                variant="ghost"
                                className="h-auto w-full items-center justify-start gap-4 rounded-md px-3 py-3 text-left transition-colors hover:bg-muted/60"
                                onClick={() => onPageChange("workspace")}
                            >
                <span className="min-w-0 flex-1">
                  <span className="block truncate font-mono text-xs font-medium">
                    {record.commandLine}
                  </span>
                  <span className="mt-1 block text-xs text-muted-foreground">
                    {record.server} · {formatTime(record.startedAt, locale)}
                  </span>
                </span>
                                <StatusPill
                                    tone={
                                        record.state === "completed"
                                            ? "good"
                                            : record.state === "failed"
                                                ? "bad"
                                                : "warn"
                                    }
                                >
                                    {stateLabel(record.state, t)}
                                </StatusPill>
                            </Button>
                        ))}
                    </div>
                )}
            </section>
            <RuntimeLogPreview
                locale={locale}
                t={t}
                onViewAll={() => onPageChange("logs")}
            />
        </div>
    );
}
