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

import {enUSMessages} from "@/i18n/locales/en-US";
import {zhCNMessages} from "@/i18n/locales/zh-CN";

export const messages = {
    "zh-CN": zhCNMessages,
    "en-US": enUSMessages,
} as const;

export type Locale = keyof typeof messages;
export type MessageKey = keyof typeof enUSMessages;
