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

import {ChevronDown, Clock3, FolderOpen, ShieldAlert, Terminal, X,} from "lucide-react";
import {type Locale, type MessageKey} from "@/i18n";
import {formatTime} from "@/utils";
import type {ApprovalDecision, PendingApproval} from "@/types";
import {Detail, Expandable} from "@/components/layout";
import {Button} from "@/components/ui/button";
import {cn} from "@/lib/utils";

export function ApprovalRows({
                                 items,
                                 t,
                                 locale,
                                 compact = false,
                                 expandedId,
                                 onDecision,
                                 onOpen,
                             }: {
    items: PendingApproval[];
    t: (key: MessageKey) => string;
    locale: Locale;
    compact?: boolean;
    expandedId?: string | null;
    onDecision?: (id: string, decision: ApprovalDecision) => void;
    onOpen?: (item: PendingApproval) => void;
}) {
    return (
        <div className="space-y-3">
            {items.map((item) => (
                <ApprovalRow
                    key={item.id}
                    item={item}
                    t={t}
                    locale={locale}
                    compact={compact}
                    expanded={expandedId === item.id}
                    onDecision={onDecision}
                    onOpen={onOpen}
                />
            ))}
        </div>
    );
}

function ApprovalRow({
                         item,
                         t,
                         locale,
                         compact,
                         expanded,
                         onDecision,
                         onOpen,
                     }: {
    item: PendingApproval;
    t: (key: MessageKey) => string;
    locale: Locale;
    compact: boolean;
    expanded: boolean;
    onDecision?: (id: string, decision: ApprovalDecision) => void;
    onOpen?: (item: PendingApproval) => void;
}) {
    return (
        <div className="rounded-md bg-card px-4 py-4 transition-colors hover:bg-muted/60 sm:p-5">
            <div className="flex items-start gap-3">
        <span className="mt-0.5 grid size-8 shrink-0 place-items-center rounded-md bg-amber-500/10 text-amber-700">
          <ShieldAlert className="size-4"/>
        </span>
                <div className="min-w-0 flex-1">
                    <Button
                        variant="ghost"
                        size="sm"
                        className="group h-auto w-full flex-col items-start justify-start gap-0 p-0 text-left hover:bg-transparent"
                        onClick={() => onOpen?.(item)}
                        aria-expanded={onOpen ? expanded : undefined}
                    >
            <span className="block truncate font-mono text-sm font-medium text-foreground group-hover:text-primary">
              {item.request.commandLine}
            </span>
                        <span
                            className="mt-2 flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-muted-foreground">
              <span>
                {t("server")}:{" "}
                  <span className="font-mono text-foreground/80">
                  {item.request.server}
                </span>
              </span>
              <span>
                {t("platform")}: {item.request.platform}
              </span>
              <span className="inline-flex items-center gap-1">
                <Clock3 className="size-3"/>
                  {formatTime(item.createdAt, locale)}
              </span>
            </span>
                    </Button>
                </div>
                {!compact && onDecision && (
                    <div className="hidden shrink-0 items-center gap-1.5 lg:flex">
                        <Button
                            variant="default"
                            size="sm"
                            className="bg-primary text-primary-foreground hover:bg-primary/90"
                            onClick={() => onDecision(item.id, "approve-once")}
                        >
                            {t("approveOnce")}
                        </Button>
                        <Button
                            variant="destructive"
                            size="sm"
                            onClick={() => onDecision(item.id, "reject")}
                        >
                            {t("reject")}
                        </Button>
                    </div>
                )}
                {onOpen && (
                    <Button
                        variant="ghost"
                        size="icon-sm"
                        onClick={() => onOpen(item)}
                        aria-label={t("openDetails")}
                        title={t("openDetails")}
                        aria-expanded={expanded}
                    >
                        <ChevronDown className={cn("size-4", expanded && "rotate-180")}/>
                    </Button>
                )}
            </div>
            {!compact && onDecision && (
                <div className="mt-4 flex flex-wrap gap-2 lg:hidden">
                    <Button
                        variant="default"
                        size="sm"
                        className="bg-primary text-primary-foreground hover:bg-primary/90"
                        onClick={() => onDecision(item.id, "approve-once")}
                    >
                        {t("approveOnce")}
                    </Button>
                    <Button
                        variant="destructive"
                        size="sm"
                        onClick={() => onDecision(item.id, "reject")}
                    >
                        {t("reject")}
                    </Button>
                </div>
            )}
            {onDecision && (
                <Expandable open={expanded} className={expanded ? "mt-5" : "mt-0"}>
                    <ApprovalDetail
                        item={item}
                        t={t}
                        locale={locale}
                        onClose={() => onOpen?.(item)}
                        onDecision={onDecision}
                    />
                </Expandable>
            )}
        </div>
    );
}

export function ApprovalDetail({
                                   item,
                                   t,
                                   locale,
                                   onClose,
                                   onDecision,
                               }: {
    item: PendingApproval;
    t: (key: MessageKey) => string;
    locale: Locale;
    onClose: () => void;
    onDecision: (id: string, decision: ApprovalDecision) => void;
}) {
    const environmentEntries = Object.entries(item.request.env);
    return (
        <div className="border-t border-border/70 pt-5">
            <div className="flex items-start justify-between gap-4">
                <div className="space-y-1">
                    <h3 className="font-heading text-sm font-semibold">
                        {t("approvalDetail")}
                    </h3>
                    <p className="text-xs text-muted-foreground">
                        {t("approvalDetailHint")}
                    </p>
                </div>
                <Button
                    variant="ghost"
                    size="icon-sm"
                    onClick={onClose}
                    aria-label={t("close")}
                    title={t("close")}
                >
                    <X className="size-4"/>
                </Button>
            </div>
            <div className="mt-5 space-y-7">
                <div className="rounded-md bg-muted/60 px-4 py-4 text-foreground">
                    <div
                        className="mb-2 flex items-center gap-2 text-[11px] uppercase tracking-[0.16em] text-muted-foreground">
                        <Terminal className="size-3"/>
                        {t("command")}
                    </div>
                    <pre className="whitespace-pre-wrap break-words font-mono text-sm leading-6">
            {item.request.commandLine}
          </pre>
                </div>
                <div className="grid gap-x-6 gap-y-5 sm:grid-cols-2">
                    <Detail label={t("server")} value={item.request.server} mono/>
                    <Detail label={t("platform")} value={item.request.platform}/>
                    <Detail
                        label={t("workingDirectory")}
                        value={item.request.workingDirectory ?? "—"}
                        mono
                    />
                    <Detail
                        label={t("defaultTimeout")}
                        value={`${item.request.timeoutMs} ms`}
                    />
                    <Detail
                        label={t("environment")}
                        value={item.request.executable}
                        mono
                    />
                    <Detail
                        label={t("arguments")}
                        value={item.request.args.join(" ") || "—"}
                        mono
                    />
                    <Detail
                        label={t("shellOperator")}
                        value={item.request.containsShellOperator ? t("yes") : t("no")}
                    />
                    <Detail
                        label={t("requestedAt")}
                        value={formatTime(item.createdAt, locale)}
                    />
                </div>
                <section>
                    <div
                        className="mb-3 flex items-center gap-2 text-xs font-semibold uppercase tracking-[0.14em] text-muted-foreground">
                        <FolderOpen className="size-3.5"/>
                        {t("environmentVariables")}
                    </div>
                    {environmentEntries.length === 0 ? (
                        <div className="rounded-lg bg-muted/50 px-3 py-3 font-mono text-xs text-muted-foreground">
                            —
                        </div>
                    ) : (
                        <dl className="space-y-2 rounded-md bg-muted/50 p-3 font-mono text-xs">
                            {environmentEntries.map(([key, value]) => (
                                <div
                                    key={key}
                                    className="grid grid-cols-[minmax(8rem,max-content)_minmax(0,1fr)] gap-3"
                                >
                                    <dt className="break-all text-muted-foreground">{key}</dt>
                                    <dd className="break-all text-foreground/80">{value}</dd>
                                </div>
                            ))}
                        </dl>
                    )}
                </section>
                {item.request.containsShellOperator && (
                    <p className="text-xs leading-5 text-amber-800">
                        {t("shellOperatorWarning")}
                    </p>
                )}
            </div>
            <div className="mt-5 flex flex-wrap gap-2">
                <Button
                    variant="default"
                    onClick={() => onDecision(item.id, "approve-once")}
                >
                    {t("approveOnce")}
                </Button>
                <Button
                    variant="destructive"
                    onClick={() => onDecision(item.id, "reject")}
                >
                    {t("reject")}
                </Button>
            </div>
        </div>
    );
}
