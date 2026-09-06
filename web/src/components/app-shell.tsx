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

import {type ComponentType, type ReactNode, useState} from "react";
import {Activity, Bell, ClipboardCheck, Languages, LogOut, Menu, ScrollText, Settings2,} from "lucide-react";
import {type Locale, type MessageKey} from "@/i18n";
import type {Page} from "@/types";
import {Button} from "@/components/ui/button";
import {useNotifications} from "@/components/notification";
import {Select, SelectContent, SelectItem, SelectTrigger, SelectValue,} from "@/components/ui/select";
import {Sheet, SheetContent, SheetHeader, SheetTitle,} from "@/components/ui/sheet";
import {cn} from "@/lib/utils";

type NavItem = {
    key: Page;
    label: MessageKey;
    icon: ComponentType<{ className?: string }>;
};

const navSections: Array<{ label: MessageKey; items: NavItem[] }> = [
    {
        label: "operations",
        items: [
            {key: "overview", label: "overview", icon: Activity},
            {key: "workspace", label: "workspace", icon: ClipboardCheck},
            {key: "logs", label: "runtimeLogs", icon: ScrollText},
        ],
    },
    {
        label: "configuration",
        items: [{key: "config", label: "configuration", icon: Settings2}],
    },
];

const nav = navSections.flatMap((section) => section.items);

function Navigation({
                        page,
                        t,
                        onNavigate,
                        className,
                    }: {
    page: Page;
    t: (key: MessageKey) => string;
    onNavigate: (page: Page) => void;
    className?: string;
}) {
    return (
        <nav
            className={cn("min-h-0 overflow-y-auto", className)}
            aria-label={t("mainNavigation")}
        >
            {navSections.map((section, index) => (
                <div key={section.label} className={cn(index > 0 && "mt-7")}>
                    <div
                        className="mb-2 px-3 text-[10px] font-semibold uppercase tracking-[0.16em] text-muted-foreground/70">
                        {t(section.label)}
                    </div>
                    <div className="space-y-1">
                        {section.items.map((item) => (
                            <NavButton
                                key={item.key}
                                item={item}
                                active={page === item.key}
                                t={t}
                                onClick={() => onNavigate(item.key)}
                            />
                        ))}
                    </div>
                </div>
            ))}
        </nav>
    );
}

export function AppShell({
                             page,
                             onPageChange,
                             onLogout,
                             apiKeyConfigured,
                             locale,
                             setLocale,
                             t,
                             children,
                         }: {
    page: Page;
    onPageChange: (page: Page) => void;
    onLogout: () => void;
    apiKeyConfigured: boolean;
    locale: Locale;
    setLocale: (locale: Locale) => void;
    t: (key: MessageKey) => string;
    children: ReactNode;
}) {
    const [mobileNavigationOpen, setMobileNavigationOpen] = useState(false);
    const {systemPermission, enableSystemNotifications} = useNotifications();
    const pageTitle = t(
        nav.find((item) => item.key === page)?.label ?? "overview",
    );
    const navigate = (nextPage: Page) => {
        onPageChange(nextPage);
        setMobileNavigationOpen(false);
    };

    return (
        <div className="min-h-dvh bg-background text-foreground">
            <aside className="fixed inset-y-0 left-0 z-30 hidden w-60 flex-col border-r border-border bg-card md:flex">
                <div className="flex h-16 shrink-0 items-center gap-3 px-5">
                    <BrandMark/>
                    <div className="min-w-0">
                        <div className="truncate font-heading text-sm font-semibold tracking-tight">
                            {t("appName")}
                        </div>
                        <div className="truncate text-xs text-muted-foreground">
                            {t("appDescriptionShort")}
                        </div>
                    </div>
                </div>
                <Navigation
                    page={page}
                    t={t}
                    onNavigate={navigate}
                    className="flex-1 px-3 py-5"
                />
            </aside>

            <div className="min-h-dvh md:pl-60">
                <header className="sticky top-0 z-20 border-b border-border/70 bg-background/95 backdrop-blur-sm">
                    <div className="mx-auto flex h-16 max-w-[1280px] items-center justify-between gap-4 px-4 sm:px-8">
                        <div className="flex min-w-0 items-center gap-3">
                            <Button
                                variant="ghost"
                                size="icon"
                                className="size-9 md:hidden"
                                aria-label={t("mainNavigation")}
                                onClick={() => setMobileNavigationOpen(true)}
                            >
                                <Menu className="size-4"/>
                            </Button>
                            <div className="min-w-0">
                                <h1 className="truncate font-heading text-lg font-semibold tracking-tight">
                                    {pageTitle}
                                </h1>
                            </div>
                        </div>
                        <div className="flex shrink-0 items-center gap-2">
                            {systemPermission === "default" && (
                                <Button
                                    variant="ghost"
                                    size="icon-sm"
                                    onClick={() => void enableSystemNotifications()}
                                    aria-label={t("enableNotifications")}
                                    title={t("enableNotifications")}
                                >
                                    <Bell className="size-3.5"/>
                                </Button>
                            )}
                            <Select
                                value={locale}
                                onValueChange={(value) => setLocale(value as Locale)}
                            >
                                <SelectTrigger
                                    size="sm"
                                    className="w-[112px] border-input bg-background"
                                    aria-label={t("language")}
                                >
                                    <Languages className="size-3.5 text-muted-foreground"/>
                                    <SelectValue/>
                                </SelectTrigger>
                                <SelectContent>
                                    <SelectItem value="zh-CN">{t("langZh")}</SelectItem>
                                    <SelectItem value="en-US">{t("langEn")}</SelectItem>
                                </SelectContent>
                            </Select>
                            {apiKeyConfigured && (
                                <Button variant="ghost" size="sm" onClick={onLogout}>
                                    <LogOut className="size-3.5"/>
                                    <span className="hidden sm:inline">{t("signOut")}</span>
                                </Button>
                            )}
                        </div>
                    </div>
                </header>

                <main className="mx-auto w-full max-w-[1280px] px-4 py-8 sm:px-8 sm:py-10">
                    {children}
                </main>
            </div>

            <Sheet open={mobileNavigationOpen} onOpenChange={setMobileNavigationOpen}>
                <SheetContent
                    side="left"
                    className="w-[min(86vw,15rem)] border-r border-border bg-card p-0"
                >
                    <SheetHeader className="h-16 justify-center px-5">
                        <SheetTitle className="flex items-center gap-3 text-left">
                            <BrandMark/>
                            <span>{t("appName")}</span>
                        </SheetTitle>
                    </SheetHeader>
                    <Navigation
                        page={page}
                        t={t}
                        onNavigate={navigate}
                        className="px-3 py-5"
                    />
                </SheetContent>
            </Sheet>
        </div>
    );
}

function BrandMark() {
    return <img src="/icon.svg" alt="" className="size-8 shrink-0"/>;
}

function NavButton({
                       item,
                       active,
                       t,
                       onClick,
                   }: {
    item: NavItem;
    active: boolean;
    t: (key: MessageKey) => string;
    onClick: () => void;
}) {
    const Icon = item.icon;
    return (
        <Button
            variant="ghost"
            size="sm"
            onClick={onClick}
            aria-current={active ? "page" : undefined}
            className={cn(
                "w-full justify-start gap-2.5 px-3 font-medium",
                active
                    ? "bg-accent text-accent-foreground hover:bg-accent hover:text-accent-foreground"
                    : "text-muted-foreground hover:bg-muted hover:text-foreground",
            )}
        >
            <Icon className="size-4"/>
            {t(item.label)}
        </Button>
    );
}
