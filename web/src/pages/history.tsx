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

import {Fragment, useEffect, useState} from "react";
import {ChevronLeft, ChevronRight, Clock3, RefreshCw, Trash2, X,} from "lucide-react";
import {apiRequest} from "@/api";
import {type Locale, type MessageKey} from "@/i18n";
import {formatTime, stateLabel} from "@/utils";
import type {HistoryPage as HistoryPageData, HistoryRecord} from "@/types";
import {ConfirmDialog} from "@/components/confirm-dialog";
import {EmptyState, ErrorState, InlineError, PageHeading, SectionHeading, StatusPill,} from "@/components/layout";
import {Button} from "@/components/ui/button";
import {Card} from "@/components/ui/card";
import {Table, TableBody, TableCell, TableHead, TableHeader, TableRow,} from "@/components/ui/table";

export function HistoryPage({
                                t,
                                locale,
                                embedded = false,
                                refreshToken = 0,
                            }: {
    t: (key: MessageKey) => string;
    locale: Locale;
    embedded?: boolean;
    refreshToken?: number;
}) {
    const limit = 25;
    const [page, setPage] = useState<HistoryPageData | null>(null);
    const [offset, setOffset] = useState(0);
    const [selected, setSelected] = useState<HistoryRecord | null>(null);
    const [output, setOutput] = useState<string | null>(null);
    const [error, setError] = useState<string | null>(null);
    const [pendingDelete, setPendingDelete] = useState<HistoryRecord | null>(
        null,
    );
    const load = (nextOffset = offset) =>
        apiRequest<HistoryPageData>(`/history?offset=${nextOffset}&limit=${limit}`)
            .then((value) => {
                setPage(value);
                setError(null);
            })
            .catch((reason: unknown) =>
                setError(reason instanceof Error ? reason.message : t("loadFailed")),
            );
    useEffect(() => {
        setOffset(0);
        load(0);
    }, [refreshToken]);

    const openRecord = async (record: HistoryRecord) => {
        if (selected?.executionId === record.executionId) {
            setSelected(null);
            setOutput(null);
            return;
        }
        setSelected(record);
        setOutput(null);
        try {
            const response = await apiRequest<{ output: string }>(
                `/history/${encodeURIComponent(record.executionId)}/output`,
            );
            setOutput(response.output);
        } catch (reason) {
            setError(reason instanceof Error ? reason.message : t("loadFailed"));
        }
    };
    const remove = (record: HistoryRecord) => setPendingDelete(record);
    const confirmRemove = async () => {
        const record = pendingDelete;
        setPendingDelete(null);
        if (!record) return;
        try {
            await apiRequest(`/history/${encodeURIComponent(record.executionId)}`, {
                method: "DELETE",
            });
            setSelected(null);
            setOutput(null);
            await load(offset);
        } catch (reason) {
            setError(reason instanceof Error ? reason.message : t("loadFailed"));
        }
    };

    if (error && !page)
        return <ErrorState message={error} onRetry={() => load(0)} t={t}/>;
    return (
        <div className={embedded ? "space-y-5" : "space-y-8"}>
            {embedded ? (
                <div>
                    <SectionHeading
                        title={t("history")}
                        action={
                            <Button variant="ghost" size="sm" onClick={() => load()}>
                                <RefreshCw className="size-3.5"/>
                                {t("refresh")}
                            </Button>
                        }
                    />
                    <p className="text-sm leading-6 text-muted-foreground">
                        {t("historyHint")}
                    </p>
                </div>
            ) : (
                <PageHeading
                    eyebrow={t("operatorWorkspace")}
                    title={t("history")}
                    description={t("historyHint")}
                    action={
                        <Button variant="outline" size="sm" onClick={() => load()}>
                            <RefreshCw className="size-3.5"/>
                            {t("refresh")}
                        </Button>
                    }
                />
            )}
            {error && <InlineError message={error}/>}
            {!page ? (
                <div className="text-sm text-muted-foreground">{t("loading")}</div>
            ) : page.records.length === 0 ? (
                <EmptyState>
                    <Clock3 className="mx-auto mb-3 size-6 text-muted-foreground"/>
                    {t("noHistory")}
                </EmptyState>
            ) : (
                <>
                    <Card className="gap-0 overflow-hidden py-0">
                        <Table>
                            <TableHeader className="[&_tr]:border-0">
                                <TableRow className="border-0 hover:bg-transparent">
                                    <TableHead className="px-4 pt-4 text-xs text-muted-foreground">
                                        {t("command")}
                                    </TableHead>
                                    <TableHead className="pt-4 text-xs text-muted-foreground">
                                        {t("server")}
                                    </TableHead>
                                    <TableHead className="pt-4 text-xs text-muted-foreground">
                                        {t("result")}
                                    </TableHead>
                                    <TableHead className="pt-4 text-xs text-muted-foreground">
                                        {t("startedAt")}
                                    </TableHead>
                                    <TableHead className="w-12 pt-4"/>
                                </TableRow>
                            </TableHeader>
                            <TableBody>
                                {page.records.map((record) => (
                                    <Fragment key={record.executionId}>
                                        <TableRow
                                            className={`cursor-pointer border-0 ${selected?.executionId === record.executionId ? "bg-primary/5" : "hover:bg-muted/50"}`}
                                            onClick={() => openRecord(record)}
                                        >
                                            <TableCell
                                                className="max-w-[360px] truncate px-4 py-4 font-mono text-xs font-medium">
                                                {record.commandLine}
                                            </TableCell>
                                            <TableCell className="font-mono text-xs text-muted-foreground">
                                                {record.server}
                                            </TableCell>
                                            <TableCell>
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
                                            </TableCell>
                                            <TableCell className="whitespace-nowrap text-xs text-muted-foreground">
                                                {formatTime(record.startedAt, locale)}
                                            </TableCell>
                                            <TableCell>
                        <span onClick={(event) => event.stopPropagation()}>
                          <Button
                              variant="ghost"
                              size="icon-sm"
                              disabled={record.state === "running"}
                              onClick={() => remove(record)}
                              aria-label={t("deleteRecord")}
                              title={t("deleteRecord")}
                          >
                            <Trash2 className="size-3.5"/>
                          </Button>
                        </span>
                                            </TableCell>
                                        </TableRow>
                                        {selected?.executionId === record.executionId && (
                                            <TableRow className="border-0 hover:bg-transparent">
                                                <TableCell colSpan={5} className="p-0">
                                                    <ExecutionDetail
                                                        record={record}
                                                        output={output}
                                                        t={t}
                                                        locale={locale}
                                                        onClose={() => {
                                                            setSelected(null);
                                                            setOutput(null);
                                                        }}
                                                    />
                                                </TableCell>
                                            </TableRow>
                                        )}
                                    </Fragment>
                                ))}
                            </TableBody>
                        </Table>
                    </Card>
                    <div className="flex items-center justify-between gap-3 text-sm text-muted-foreground">
            <span>
              {page.total} {t("records")}
            </span>
                        <div className="flex gap-1">
                            <Button
                                variant="ghost"
                                size="sm"
                                disabled={page.offset === 0}
                                onClick={() => {
                                    const next = Math.max(0, page.offset - limit);
                                    setOffset(next);
                                    load(next);
                                }}
                            >
                                <ChevronLeft className="size-3.5"/>
                                {t("previous")}
                            </Button>
                            <Button
                                variant="ghost"
                                size="sm"
                                disabled={page.offset + page.limit >= page.total}
                                onClick={() => {
                                    const next = page.offset + limit;
                                    setOffset(next);
                                    load(next);
                                }}
                            >
                                {t("next")}
                                <ChevronRight className="size-3.5"/>
                            </Button>
                        </div>
                    </div>
                </>
            )}
            <ConfirmDialog
                open={Boolean(pendingDelete)}
                title={t("deleteConfirmTitle")}
                description={t("deleteConfirmDescription")}
                confirmLabel={t("deleteRecord")}
                cancelLabel={t("cancel")}
                onCancel={() => setPendingDelete(null)}
                onConfirm={confirmRemove}
            />
        </div>
    );
}

function ExecutionDetail({
                             record,
                             output,
                             t,
                             locale,
                             onClose,
                         }: {
    record: HistoryRecord;
    output: string | null;
    t: (key: MessageKey) => string;
    locale: Locale;
    onClose: () => void;
}) {
    return (
        <div
            className="border-l-2 border-primary/40 bg-muted/20 px-4 py-5 animate-in fade-in-0 slide-in-from-top-2 duration-200 sm:px-6">
            <div className="flex flex-row items-start justify-between gap-4">
                <div className="min-w-0 space-y-1">
                    <h3 className="font-heading text-sm font-semibold">
                        {t("executionDetails")}
                    </h3>
                    <p className="truncate font-mono text-xs text-muted-foreground">
                        {record.commandLine}
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
                <div className="grid gap-x-6 gap-y-5 sm:grid-cols-2 lg:grid-cols-3">
                    <div className="sm:col-span-2 lg:col-span-3">
                        <div className="mb-2 text-xs text-muted-foreground">
                            {t("command")}
                        </div>
                        <pre
                            className="whitespace-pre-wrap break-words rounded-md bg-muted px-4 py-4 font-mono text-xs leading-6 text-foreground">
              {record.commandLine}
            </pre>
                    </div>
                    <div>
                        <div className="text-xs text-muted-foreground">{t("server")}</div>
                        <div className="mt-1 font-mono text-sm">{record.server}</div>
                    </div>
                    <div>
                        <div className="text-xs text-muted-foreground">
                            {t("startedAt")}
                        </div>
                        <div className="mt-1 text-sm">
                            {formatTime(record.startedAt, locale)}
                        </div>
                    </div>
                    <div>
                        <div className="text-xs text-muted-foreground">
                            {t("finishedAt")}
                        </div>
                        <div className="mt-1 text-sm">
                            {formatTime(record.finishedAt, locale)}
                        </div>
                    </div>
                    <div>
                        <div className="text-xs text-muted-foreground">{t("exitCode")}</div>
                        <div className="mt-1 text-sm">
                            {record.exitCode === null ? "—" : record.exitCode}
                        </div>
                    </div>
                    <div>
                        <div className="text-xs text-muted-foreground">{t("result")}</div>
                        <div className="mt-1 text-sm">{stateLabel(record.state, t)}</div>
                    </div>
                </div>
                <div>
                    <div className="mb-2 text-xs font-semibold uppercase tracking-[0.14em] text-muted-foreground">
                        {t("output")}
                    </div>
                    <pre
                        className="max-h-[calc(100vh-23rem)] min-h-48 overflow-auto whitespace-pre-wrap break-words rounded-md bg-muted p-4 font-mono text-xs leading-5 text-foreground/85">
            {output ?? t("loading")}
          </pre>
                </div>
            </div>
        </div>
    );
}
