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

import {type FormEvent, useState} from "react";
import {Languages, LockKeyhole} from "lucide-react";
import {apiRequest, ApiRequestError} from "@/api";
import {type Locale, type MessageKey} from "@/i18n";
import {Field} from "@/components/form";
import {InlineError} from "@/components/layout";
import {Button} from "@/components/ui/button";
import {Card, CardContent, CardDescription, CardHeader, CardTitle,} from "@/components/ui/card";
import {Select, SelectContent, SelectItem, SelectTrigger, SelectValue,} from "@/components/ui/select";

export function LoginScreen({
                                locale,
                                setLocale,
                                t,
                                onLogin,
                                showBootstrapExpired,
                            }: {
    locale: Locale;
    setLocale: (locale: Locale) => void;
    t: (key: MessageKey) => string;
    onLogin: () => void;
    showBootstrapExpired: boolean;
}) {
    const [apiKey, setApiKey] = useState("");
    const [error, setError] = useState<string | null>(null);
    const submit = async (event: FormEvent<HTMLFormElement>) => {
        event.preventDefault();
        setError(null);
        try {
            await apiRequest("/session/login", {
                method: "POST",
                body: JSON.stringify({apiKey}),
            });
            onLogin();
        } catch (reason) {
            setError(
                reason instanceof ApiRequestError
                    ? t("invalidApiKey")
                    : reason instanceof Error
                        ? reason.message
                        : t("invalidApiKey"),
            );
        }
    };
    return (
        <main className="grid min-h-screen place-items-center bg-background px-4 py-8">
            <div className="w-full max-w-md animate-in fade-in-0 slide-in-from-bottom-2 duration-500">
                <div className="mb-8 flex items-center justify-between">
                    <div className="flex items-center gap-3">
                        <img src="/icon.svg" alt="" className="size-10"/>
                        <div>
                            <div className="font-heading text-sm font-semibold">
                                {t("appName")}
                            </div>
                            <div className="text-xs text-muted-foreground">
                                {t("appDescriptionShort")}
                            </div>
                        </div>
                    </div>
                    <Select
                        value={locale}
                        onValueChange={(value) => setLocale(value as Locale)}
                    >
                        <SelectTrigger
                            size="sm"
                            className="w-[104px] border-input bg-background"
                        >
                            <Languages className="size-3.5 text-muted-foreground"/>
                            <SelectValue/>
                        </SelectTrigger>
                        <SelectContent>
                            <SelectItem value="zh-CN">{t("langZh")}</SelectItem>
                            <SelectItem value="en-US">{t("langEn")}</SelectItem>
                        </SelectContent>
                    </Select>
                </div>
                <Card className="animate-in fade-in-0 slide-in-from-bottom-1 duration-500">
                    <CardHeader className="gap-4 px-6 pt-6 sm:px-8 sm:pt-8">
            <span className="grid size-10 place-items-center rounded-lg bg-primary/10 text-primary">
              <LockKeyhole className="size-5"/>
            </span>
                        <CardTitle className="text-2xl tracking-tight">
                            {t("loginTitle")}
                        </CardTitle>
                        <CardDescription className="leading-6">
                            {t("loginDescription")}
                        </CardDescription>
                        {showBootstrapExpired && (
                            <p className="mt-4 rounded-lg bg-amber-50 px-3 py-2 text-xs leading-5 text-amber-900">
                                {t("bootstrapExpired")}
                            </p>
                        )}
                    </CardHeader>
                    <CardContent className="px-6 pb-6 sm:px-8 sm:pb-8">
                        <form onSubmit={submit} className="space-y-5">
                            <Field
                                label={t("apiKey")}
                                value={apiKey}
                                onChange={setApiKey}
                                mono
                                type="password"
                                placeholder="••••••••"
                            />
                            {error && <InlineError message={error}/>}
                            <Button
                                type="submit"
                                variant="default"
                                className="w-full bg-primary text-primary-foreground hover:bg-primary/90"
                                disabled={!apiKey}
                            >
                                {t("login")}
                            </Button>
                        </form>
                    </CardContent>
                </Card>
            </div>
        </main>
    );
}
