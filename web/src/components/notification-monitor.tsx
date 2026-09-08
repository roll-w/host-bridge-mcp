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
import {apiRequest, parseEvent} from "@/api";
import {useNotifications} from "@/components/notification";
import {type MessageKey} from "@/i18n";

export const APPROVALS_CHANGED_EVENT = "host-bridge:approvals-changed";

type ApprovalSnapshot = {
    items: Array<{ id: string }>;
};

type ApprovalEvent =
    | { type: "created"; approval: { id: string } }
    | { type: "removed"; id: string };

export function NotificationMonitor({t}: { t: (key: MessageKey) => string }) {
    const previousApprovalIds = useRef<Set<string> | null>(null);
    const {notify} = useNotifications();

    useEffect(() => {
        let active = true;
        const source = new EventSource("/api/v1/approvals/event");
        const publishChange = () => {
            window.dispatchEvent(new Event(APPROVALS_CHANGED_EVENT));
        };
        const updateApprovalIds = (nextApprovalIds: Set<string>) => {
            const previous = previousApprovalIds.current;
            if (previous && [...nextApprovalIds].some((id) => !previous.has(id))) {
                notify({message: t("newApprovalNotification")});
            }
            previousApprovalIds.current = nextApprovalIds;
            publishChange();
        };
        const resync = async () => {
            try {
                const data = await apiRequest<ApprovalSnapshot>("/approvals");
                if (active) updateApprovalIds(new Set(data.items.map((item) => item.id)));
            } catch {
                // The stream will reconnect automatically after a transient failure.
            }
        };

        source.addEventListener("snapshot", (event) => {
            const data = parseEvent<ApprovalSnapshot>(event);
            if (active && data) updateApprovalIds(new Set(data.items.map((item) => item.id)));
        });
        source.addEventListener("approval", (event) => {
            const data = parseEvent<ApprovalEvent>(event);
            if (!active || !data) return;
            const nextApprovalIds = new Set(previousApprovalIds.current ?? []);
            if (data.type === "created") {
                if (!nextApprovalIds.has(data.approval.id)) {
                    notify({message: t("newApprovalNotification")});
                }
                nextApprovalIds.add(data.approval.id);
            } else {
                nextApprovalIds.delete(data.id);
            }
            previousApprovalIds.current = nextApprovalIds;
            publishChange();
        });
        source.addEventListener("lagged", () => {
            if (active) void resync();
        });
        return () => {
            active = false;
            source.close();
        };
    }, [notify, t]);

    return null;
}
