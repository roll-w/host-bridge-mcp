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

import {useState} from "react";
import {type Locale, type MessageKey, messages} from "@/i18n/messages";

export {messages, type Locale, type MessageKey} from "@/i18n/messages";

export function useLocale(): [Locale, (locale: Locale) => void] {
    const [locale, setLocale] = useState<Locale>(() => {
        const stored = window.localStorage.getItem("host-bridge-locale");
        if (stored === "zh-CN" || stored === "en-US") return stored;
        return navigator.language.toLowerCase().startsWith("zh")
            ? "zh-CN"
            : "en-US";
    });

    const update = (next: Locale) => {
        setLocale(next);
        window.localStorage.setItem("host-bridge-locale", next);
    };

    return [locale, update];
}

export function useMessages(locale: Locale): (key: MessageKey) => string {
    return (key: MessageKey) => messages[locale][key] ?? messages["en-US"][key];
}
