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
import {apiRequest} from "@/api";
import {PageHeading} from "@/components/layout";
import {RunningExecutionsPanel} from "@/components/running-execution";
import {type Locale, type MessageKey} from "@/i18n";
import type {ApprovalDecision, HistoryPage as HistoryPageData, PendingApproval,} from "@/types";
import {ApprovalsPage} from "@/pages/approvals";
import {HistoryPage} from "@/pages/history";

export function WorkspacePage({
                                  t,
                                  locale,
                              }: {
    t: (key: MessageKey) => string;
    locale: Locale;
}) {
    const [runningExecutionIds, setRunningExecutionIds] = useState<string[]>([]);
    const [historyRefreshToken, setHistoryRefreshToken] = useState(0);
    const protectedExecutionIds = useRef(new Map<string, number>());

    useEffect(() => {
        let active = true;

        const loadRunningExecutions = async () => {
            try {
                const page = await apiRequest<HistoryPageData>(
                    "/history?offset=0&limit=1000",
                );
                if (!active) return;

                const now = Date.now();
                const runningIds = page.records
                    .filter((record) => record.state === "running")
                    .map((record) => record.executionId);
                const runningSet = new Set(runningIds);

                for (const [executionId, expiresAt] of protectedExecutionIds.current) {
                    if (expiresAt <= now || runningSet.has(executionId)) {
                        protectedExecutionIds.current.delete(executionId);
                    }
                }

                setRunningExecutionIds((current) => {
                    const next = [
                        ...runningIds,
                        ...current.filter((executionId) => {
                            const expiresAt = protectedExecutionIds.current.get(executionId);
                            return (
                                runningSet.has(executionId) ||
                                (expiresAt !== undefined && expiresAt > now)
                            );
                        }),
                    ];
                    const unique = [...new Set(next)];
                    if (
                        unique.length === current.length &&
                        unique.every((executionId, index) => executionId === current[index])
                    ) {
                        return current;
                    }
                    return unique;
                });
            } catch {
                // Keep the current execution list visible while the next poll retries.
            }
        };

        void loadRunningExecutions();
        const timer = window.setInterval(loadRunningExecutions, 1_500);
        return () => {
            active = false;
            window.clearInterval(timer);
        };
    }, []);

    const handleApprovalResolved = (
        item: PendingApproval,
        decision: ApprovalDecision,
    ) => {
        if (decision === "reject") return;
        protectedExecutionIds.current.set(item.executionId, Date.now() + 5_000);
        setRunningExecutionIds((current) =>
            current.includes(item.executionId)
                ? current
                : [...current, item.executionId],
        );
        setHistoryRefreshToken((value) => value + 1);
    };

    return (
        <div className="space-y-10">
            <PageHeading
                eyebrow={t("operations")}
                title={t("workspace")}
                description={t("workspaceHint")}
            />
            <section className="grid items-start gap-10 xl:grid-cols-[minmax(0,1fr)_minmax(20rem,0.72fr)]">
                <div className="min-w-0 space-y-10">
                    <ApprovalsPage
                        t={t}
                        locale={locale}
                        embedded
                        onResolved={handleApprovalResolved}
                    />
                    <HistoryPage
                        t={t}
                        locale={locale}
                        embedded
                        refreshToken={historyRefreshToken}
                    />
                </div>
                <aside className="min-w-0">
                    <RunningExecutionsPanel executionIds={runningExecutionIds} t={t}/>
                </aside>
            </section>
        </div>
    );
}
