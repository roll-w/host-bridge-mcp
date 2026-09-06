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

import type {ReactNode} from "react";
import {Badge} from "@/components/ui/badge";
import {Button} from "@/components/ui/button";
import {Skeleton} from "@/components/ui/skeleton";
import {cn} from "@/lib/utils";

export function PageHeading({
                                eyebrow,
                                title,
                                description,
                                action,
                            }: {
    eyebrow?: string;
    title: string;
    description?: string;
    action?: ReactNode;
}) {
    return (
        <div className="flex flex-wrap items-end justify-between gap-4">
            <div className="min-w-0">
                {eyebrow && (
                    <div className="mb-2 text-[11px] font-semibold uppercase tracking-[0.18em] text-primary/80">
                        {eyebrow}
                    </div>
                )}
                <h1 className="font-heading text-2xl font-semibold tracking-tight text-foreground sm:text-3xl">
                    {title}
                </h1>
                {description && (
                    <p className="mt-2 max-w-3xl text-sm leading-6 text-muted-foreground">
                        {description}
                    </p>
                )}
            </div>
            {action}
        </div>
    );
}

export function StatusPill({
                               children,
                               tone = "neutral",
                               className,
                           }: {
    children: ReactNode;
    tone?: "good" | "warn" | "bad" | "neutral";
    className?: string;
}) {
    const toneClass = {
        good: "bg-emerald-500/10 text-emerald-700",
        warn: "bg-amber-500/10 text-amber-800",
        bad: "bg-destructive/10 text-destructive",
        neutral: "bg-muted text-muted-foreground",
    }[tone];
    return (
        <Badge
            variant="secondary"
            className={cn(
                "rounded-full px-2.5 py-1 font-medium",
                toneClass,
                className,
            )}
        >
            <span className="size-1.5 rounded-full bg-current"/>
            {children}
        </Badge>
    );
}

export function EmptyState({
                               children,
                               className,
                           }: {
    children: ReactNode;
    className?: string;
}) {
    return (
        <div
            className={cn(
                "rounded-lg bg-muted/60 px-5 py-12 text-center text-sm text-muted-foreground",
                className,
            )}
        >
            {children}
        </div>
    );
}

export function LoadingState({t}: { t: (key: "loading") => string }) {
    return (
        <div className="space-y-4">
            <Skeleton className="h-8 w-44 bg-primary/10"/>
            <Skeleton className="h-4 w-72 bg-primary/5"/>
            <Skeleton className="h-40 w-full bg-primary/5"/>
            <span className="sr-only">{t("loading")}</span>
        </div>
    );
}

export function InlineError({message}: { message: string }) {
    return (
        <div
            role="alert"
            className="rounded-lg bg-destructive/10 px-4 py-3 text-sm text-destructive"
        >
            {message}
        </div>
    );
}

export function ErrorState({
                               message,
                               onRetry,
                               t,
                           }: {
    message: string;
    onRetry: () => void;
    t: (key: "retry") => string;
}) {
    return (
        <div className="space-y-4">
            <InlineError message={message}/>
            <Button variant="outline" onClick={onRetry}>
                {t("retry")}
            </Button>
        </div>
    );
}

export function Metric({
                           label,
                           value,
                           detail,
                           mono = false,
                           onClick,
                       }: {
    label: string;
    value: ReactNode;
    detail?: ReactNode;
    mono?: boolean;
    onClick?: () => void;
}) {
    const content = (
        <>
            <div className="text-xs font-medium text-muted-foreground">{label}</div>
            <div
                className={cn(
                    "mt-2 text-xl font-semibold tracking-tight",
                    mono && "font-mono text-base",
                )}
            >
                {value}
            </div>
            {detail && (
                <div className="mt-1 text-xs text-muted-foreground">{detail}</div>
            )}
        </>
    );
    return onClick ? (
        <Button
            variant="ghost"
            className="h-auto w-full min-w-0 flex-col items-start justify-start p-4 text-left transition-colors hover:bg-muted/60 sm:p-5"
            onClick={onClick}
        >
            {content}
        </Button>
    ) : (
        <div className="min-w-0 p-4 sm:p-5">{content}</div>
    );
}

export function SectionHeading({
                                   title,
                                   action,
                                   plain = false,
                               }: {
    title: string;
    action?: ReactNode;
    plain?: boolean;
}) {
    return (
        <div
            className={cn(
                "flex items-center justify-between gap-3",
                !plain && "mb-4",
            )}
        >
            <h2 className="font-heading text-base font-semibold tracking-tight">
                {title}
            </h2>
            {action}
        </div>
    );
}

export function Expandable({
                               open,
                               children,
                               className,
                           }: {
    open: boolean;
    children: ReactNode;
    className?: string;
}) {
    return (
        <div
            aria-hidden={!open}
            className={cn(
                "grid overflow-hidden transition-[grid-template-rows,opacity,visibility,margin] duration-200 ease-out",
                open
                    ? "visible grid-rows-[1fr] opacity-100"
                    : "invisible pointer-events-none grid-rows-[0fr] opacity-0",
                className,
            )}
        >
            <div className="min-h-0 overflow-hidden">{children}</div>
        </div>
    );
}

export function ConfigGroup({
                                title,
                                description,
                                children,
                            }: {
    title: string;
    description?: string;
    children: ReactNode;
}) {
    return (
        <section className="space-y-4">
            <div>
                <h2 className="font-heading text-base font-semibold tracking-tight">
                    {title}
                </h2>
                {description && (
                    <p className="mt-1 text-sm leading-6 text-muted-foreground">
                        {description}
                    </p>
                )}
            </div>
            {children}
        </section>
    );
}

export function Detail({
                           label,
                           value,
                           mono = false,
                       }: {
    label: string;
    value: string;
    mono?: boolean;
}) {
    return (
        <div className="min-w-0">
            <div className="text-xs text-muted-foreground">{label}</div>
            <div
                className={cn("mt-1 break-words text-sm", mono && "font-mono text-xs")}
            >
                {value}
            </div>
        </div>
    );
}
