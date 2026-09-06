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

import type {Locale, MessageKey} from "@/i18n";

export function formatTime(
    value: string | number | null,
    locale: Locale,
): string {
    if (value === null) return "—";
    const date = new Date(value);
    if (Number.isNaN(date.getTime())) return String(value);
    return new Intl.DateTimeFormat(locale, {
        dateStyle: "short",
        timeStyle: "medium",
    }).format(date);
}

export function stateLabel(
    state: string,
    t: (key: MessageKey) => string,
): string {
    const labels: Record<string, MessageKey> = {
        running: "stateRunning",
        completed: "stateCompleted",
        failed: "stateFailed",
    };
    return t(labels[state] ?? "stateUnknown");
}
