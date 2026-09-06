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

import {useEffect, useRef} from "react";
import {apiRequest} from "@/api";
import {useNotifications} from "@/components/notification";
import {type MessageKey} from "@/i18n";

type ApprovalList = {
    items: Array<{ id: string }>;
};

type HistoryList = {
    records: Array<{
        executionId: string;
        state: "running" | "completed" | "failed";
    }>;
};

export function NotificationMonitor({t}: { t: (key: MessageKey) => string }) {
    const previousApprovalIds = useRef<Set<string> | null>(null);
    const previousRunningIds = useRef<Set<string> | null>(null);
    const {notify} = useNotifications();

    useEffect(() => {
        let active = true;

        const loadNotifications = async () => {
            const [approvalResult, historyResult] = await Promise.allSettled([
                apiRequest<ApprovalList>("/approvals"),
                apiRequest<HistoryList>("/history?offset=0&limit=1000"),
            ]);
            if (!active) return;

            if (approvalResult.status === "fulfilled") {
                const approvalData = approvalResult.value;
                const nextApprovalIds = new Set(
                    approvalData.items.map((item) => item.id),
                );
                const previous = previousApprovalIds.current;
                if (previous && [...nextApprovalIds].some((id) => !previous.has(id))) {
                    notify({message: t("newApprovalNotification")});
                }
                previousApprovalIds.current = nextApprovalIds;
            }

            if (historyResult.status === "fulfilled") {
                const historyData = historyResult.value;
                const nextRunningIds = new Set(
                    historyData.records
                        .filter((record) => record.state === "running")
                        .map((record) => record.executionId),
                );
                const previousRunning = previousRunningIds.current;
                const finishedCount = previousRunning
                    ? [...previousRunning].filter((id) => !nextRunningIds.has(id)).length
                    : 0;
                if (finishedCount > 0) {
                    notify({
                        message: t(
                            finishedCount === 1
                                ? "executionFinishedNotification"
                                : "executionsFinishedNotification",
                        ),
                        tone: "success",
                    });
                }
                previousRunningIds.current = nextRunningIds;
            }
        };

        void loadNotifications();
        const timer = window.setInterval(loadNotifications, 1_500);
        return () => {
            active = false;
            window.clearInterval(timer);
        };
    }, [notify, t]);

    return null;
}
