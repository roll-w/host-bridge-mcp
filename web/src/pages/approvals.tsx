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
import {RefreshCw, ShieldCheck} from "lucide-react";
import {apiRequest, jsonBody} from "@/api";
import {type Locale, type MessageKey} from "@/i18n";
import type {ApprovalDecision, PendingApproval} from "@/types";
import {ApprovalRows} from "@/components/approval";
import {EmptyState, ErrorState, InlineError, PageHeading, SectionHeading,} from "@/components/layout";
import {Button} from "@/components/ui/button";
import {useNotifications} from "@/components/notification";

export function ApprovalsPage({
                                  t,
                                  locale,
                                  embedded = false,
                                  onResolved,
                              }: {
    t: (key: MessageKey) => string;
    locale: Locale;
    embedded?: boolean;
    onResolved?: (item: PendingApproval, decision: ApprovalDecision) => void;
}) {
    const [items, setItems] = useState<PendingApproval[]>([]);
    const [interactive, setInteractive] = useState(false);
    const [selected, setSelected] = useState<PendingApproval | null>(null);
    const [error, setError] = useState<string | null>(null);
    const {notify} = useNotifications();

    const load = () =>
        apiRequest<{ items: PendingApproval[]; interactive: boolean }>("/approvals")
            .then((data) => {
                setItems(data.items);
                setInteractive(data.interactive);
                setSelected((current) =>
                    current && data.items.some((item) => item.id === current.id)
                        ? current
                        : null,
                );
                setError(null);
            })
            .catch((reason: unknown) =>
                setError(reason instanceof Error ? reason.message : t("loadFailed")),
            );
    useEffect(() => {
        load();
        const timer = window.setInterval(load, 2000);
        return () => window.clearInterval(timer);
    }, []);

    const decide = async (id: string, decision: ApprovalDecision) => {
        try {
            await apiRequest(
                `/approvals/${encodeURIComponent(id)}`,
                jsonBody({decision}),
            );
            const item = items.find((value) => value.id === id);
            if (item) onResolved?.(item, decision);
            notify({
                message:
                    decision === "reject"
                        ? t("approvalRejectedNotification")
                        : t("approvalApprovedNotification"),
                tone: decision === "reject" ? "info" : "success",
            });
            setSelected(null);
            await load();
        } catch (reason) {
            const message = reason instanceof Error ? reason.message : t("loadFailed");
            setError(message);
            notify({message, tone: "error"});
        }
    };

    if (error && items.length === 0)
        return <ErrorState message={error} onRetry={load} t={t}/>;
    return (
        <div className={embedded ? "space-y-5" : "space-y-9"}>
            {embedded ? (
                <div>
                    <SectionHeading
                        title={t("approvals")}
                        action={
                            <Button variant="ghost" size="sm" onClick={load}>
                                <RefreshCw className="size-3.5"/>
                                {t("refresh")}
                            </Button>
                        }
                    />
                    <p className="text-sm leading-6 text-muted-foreground">
                        {interactive ? t("approvalHint") : t("offline")}
                    </p>
                </div>
            ) : (
                <PageHeading
                    eyebrow={t("operatorWorkspace")}
                    title={t("approvals")}
                    description={interactive ? t("approvalHint") : t("offline")}
                    action={
                        <Button variant="outline" size="sm" onClick={load}>
                            <RefreshCw className="size-3.5"/>
                            {t("refresh")}
                        </Button>
                    }
                />
            )}
            {error && <InlineError message={error}/>}
            {items.length === 0 ? (
                <EmptyState>
                    <ShieldCheck className="mx-auto mb-3 size-6 text-emerald-600"/>
                    {t("noPendingApprovals")}
                </EmptyState>
            ) : (
                <ApprovalRows
                    items={items}
                    t={t}
                    locale={locale}
                    onDecision={decide}
                    expandedId={selected?.id}
                    onOpen={(item) =>
                        setSelected((current) => (current?.id === item.id ? null : item))
                    }
                />
            )}
        </div>
    );
}
