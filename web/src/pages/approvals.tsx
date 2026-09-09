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
import {RefreshCw, ShieldCheck} from "lucide-react";
import {apiRequest, jsonBody} from "@/api";
import {type Locale, type MessageKey} from "@/i18n";
import type {ApprovalDecision, PendingApproval} from "@/types";
import {ApprovalRows, ApprovalShortcutLegend} from "@/components/approval";
import {EmptyState, ErrorState, InlineError, PageHeading, SectionHeading,} from "@/components/layout";
import {APPROVALS_CHANGED_EVENT} from "@/components/notification-monitor";
import {Button} from "@/components/ui/button";

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
    const [approvalAvailable, setApprovalAvailable] = useState(false);
    const [selected, setSelected] = useState<PendingApproval | null>(null);
    const [activeId, setActiveId] = useState<string | null>(null);
    const [error, setError] = useState<string | null>(null);
    const approvalsRef = useRef<HTMLDivElement | null>(null);
    const decidingId = useRef<string | null>(null);

    const load = () =>
        apiRequest<{ items: PendingApproval[]; approvalAvailable: boolean }>("/approvals")
            .then((data) => {
                setItems(data.items);
                setApprovalAvailable(data.approvalAvailable);
                setActiveId((current) =>
                    current && data.items.some((item) => item.id === current)
                        ? current
                        : data.items[0]?.id ?? null,
                );
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
        void load();
        const handleApprovalsChanged = () => {
            void load();
        };
        window.addEventListener(APPROVALS_CHANGED_EVENT, handleApprovalsChanged);
        return () =>
            window.removeEventListener(APPROVALS_CHANGED_EVENT, handleApprovalsChanged);
    }, []);

    const decide = async (id: string, decision: ApprovalDecision) => {
        if (decidingId.current !== null) return;
        decidingId.current = id;
        const itemIndex = items.findIndex((item) => item.id === id);
        const nextItem =
            items[itemIndex + 1] ?? items[itemIndex - 1] ?? items[itemIndex];
        setActiveId(nextItem?.id ?? id);
        try {
            await apiRequest(
                `/approvals/${encodeURIComponent(id)}`,
                jsonBody({decision}),
            );
            const item = items.find((value) => value.id === id);
            if (item) onResolved?.(item, decision);
            setSelected(null);
            await load();
        } catch (reason) {
            setActiveId(id);
            const message = reason instanceof Error ? reason.message : t("loadFailed");
            setError(message);
        } finally {
            decidingId.current = null;
        }
    };

    useEffect(() => {
        const handleKeyDown = (event: KeyboardEvent) => {
            const target = event.target;
            const isButtonLikeTarget =
                target instanceof HTMLElement &&
                target.closest("button, a, [role='button']") !== null;
            if (
                target instanceof Node &&
                approvalsRef.current &&
                !approvalsRef.current.contains(target) &&
                isButtonLikeTarget
            ) {
                return;
            }
            if (
                target instanceof HTMLElement &&
                (target.isContentEditable ||
                    target instanceof HTMLInputElement ||
                    target instanceof HTMLTextAreaElement ||
                    target instanceof HTMLSelectElement)
            ) {
                return;
            }
            if (
                event.defaultPrevented ||
                event.isComposing ||
                event.altKey ||
                event.ctrlKey ||
                event.metaKey ||
                items.length === 0
            ) {
                return;
            }

            const currentIndex = Math.max(
                0,
                items.findIndex((item) => item.id === activeId),
            );
            const move = (offset: number) => {
                const nextIndex = Math.min(
                    items.length - 1,
                    Math.max(0, currentIndex + offset),
                );
                setActiveId(items[nextIndex].id);
            };

            if (event.key === "ArrowDown" || event.key.toLowerCase() === "j") {
                event.preventDefault();
                move(1);
                return;
            }
            if (event.key === "ArrowUp" || event.key.toLowerCase() === "k") {
                event.preventDefault();
                move(-1);
                return;
            }

            const activeItem = items[currentIndex];
            if (!activeItem) return;

            if (event.key.toLowerCase() === "a") {
                event.preventDefault();
                void decide(activeItem.id, "approve-once");
                return;
            }
            if (event.key.toLowerCase() === "r" || event.key === "Delete") {
                event.preventDefault();
                void decide(activeItem.id, "reject");
                return;
            }
            if (event.key === "Enter" && !isButtonLikeTarget) {
                event.preventDefault();
                setSelected((current) =>
                    current?.id === activeItem.id ? null : activeItem,
                );
                return;
            }
            if (event.key === "Escape" && selected !== null) {
                event.preventDefault();
                setSelected(null);
            }
        };

        window.addEventListener("keydown", handleKeyDown);
        return () => window.removeEventListener("keydown", handleKeyDown);
    }, [activeId, decide, items, selected]);

    if (error && items.length === 0)
        return <ErrorState message={error} onRetry={load} t={t}/>;
    return (
        <div
            ref={approvalsRef}
            className={embedded ? "space-y-5" : "space-y-9"}
        >
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
                        {approvalAvailable ? t("approvalHint") : t("offline")}
                    </p>
                    <div className="mt-3 rounded-md border border-border/60 bg-muted/30 px-3 py-2">
                        <ApprovalShortcutLegend t={t}/>
                    </div>
                </div>
            ) : (
                <>
                    <PageHeading
                        eyebrow={t("operatorWorkspace")}
                        title={t("approvals")}
                        description={
                            approvalAvailable ? t("approvalHint") : t("offline")
                        }
                        action={
                            <Button variant="outline" size="sm" onClick={load}>
                                <RefreshCw className="size-3.5"/>
                                {t("refresh")}
                            </Button>
                        }
                    />
                    <div className="mt-3 rounded-md border border-border/60 bg-muted/30 px-3 py-2">
                        <ApprovalShortcutLegend t={t}/>
                    </div>
                </>
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
                    activeId={activeId}
                    onActiveChange={setActiveId}
                    expandedId={selected?.id}
                    onOpen={(item) => {
                        setActiveId(item.id);
                        setSelected((current) =>
                            current?.id === item.id ? null : item,
                        );
                    }}
                />
            )}
        </div>
    );
}
