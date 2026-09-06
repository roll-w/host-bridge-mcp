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
import {apiRequest} from "@/api";
import {AppShell} from "@/components/app-shell";
import {LoginScreen} from "@/components/login-screen";
import {LoadingState} from "@/components/layout";
import {TooltipProvider} from "@/components/ui/tooltip";
import {useLocale, useMessages} from "@/i18n";
import {ConfigurationPage} from "@/pages/configuration";
import {LogsPage} from "@/pages/logs";
import {OverviewPage} from "@/pages/overview";
import {WorkspacePage} from "@/pages/workspace";
import type {Page} from "@/types";

type SessionStatus = {
    authenticated: boolean;
    apiKeyConfigured: boolean;
};

export default function App() {
    const [locale, setLocale] = useLocale();
    const t = useMessages(locale);
    const [page, setPage] = useState<Page>("overview");
    const [session, setSession] = useState<"checking" | "ready" | "login">(
        "checking",
    );
    const [apiKeyConfigured, setApiKeyConfigured] = useState(false);
    const [bootstrapExpired, setBootstrapExpired] = useState(false);

    useEffect(() => {
        const hash = new URLSearchParams(window.location.hash.replace(/^#/, ""));
        const bootstrap = hash.get("bootstrap");
        const check: Promise<SessionStatus> = bootstrap
            ? apiRequest<SessionStatus>("/session/exchange", {
                method: "POST",
                body: JSON.stringify({bootstrapToken: bootstrap}),
            })
            : apiRequest<SessionStatus>("/session/status");
        check
            .then((status) => {
                setApiKeyConfigured(status.apiKeyConfigured);
                if (!bootstrap && status.apiKeyConfigured && !status.authenticated) {
                    setSession("login");
                    return;
                }
                if (bootstrap)
                    window.history.replaceState(
                        null,
                        "",
                        `${window.location.pathname}${window.location.search}`,
                    );
                setSession("ready");
            })
            .catch(() => {
                setBootstrapExpired(Boolean(bootstrap));
                setSession("login");
            });
    }, []);

    if (session === "checking") return <LoadingState t={t}/>;
    if (session === "login")
        return (
            <LoginScreen
                locale={locale}
                setLocale={setLocale}
                t={t}
                onLogin={() => {
                    setApiKeyConfigured(true);
                    setSession("ready");
                }}
                showBootstrapExpired={bootstrapExpired}
            />
        );

    const content =
        page === "overview" ? (
            <OverviewPage t={t} locale={locale} onPageChange={setPage}/>
        ) : page === "workspace" ? (
            <WorkspacePage t={t} locale={locale}/>
        ) : page === "logs" ? (
            <LogsPage t={t} locale={locale}/>
        ) : (
            <ConfigurationPage t={t}/>
        );
    return (
        <TooltipProvider>
            <AppShell
                page={page}
                onPageChange={setPage}
                onLogout={async () => {
                    await apiRequest("/session/logout", {method: "POST"}).catch(
                        () => undefined,
                    );
                    setBootstrapExpired(false);
                    setSession(apiKeyConfigured ? "login" : "ready");
                }}
                apiKeyConfigured={apiKeyConfigured}
                locale={locale}
                setLocale={setLocale}
                t={t}
            >
                <div key={page} className="animate-in fade-in-0 duration-200">
                    {content}
                </div>
            </AppShell>
        </TooltipProvider>
    );
}
