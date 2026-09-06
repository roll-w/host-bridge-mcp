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

import {createContext, type ReactNode, useCallback, useContext, useEffect, useRef, useState,} from "react";
import {CheckCircle2, CircleAlert, Info, X} from "lucide-react";
import {Alert, AlertDescription} from "@/components/ui/alert";
import {Button} from "@/components/ui/button";
import {cn} from "@/lib/utils";

export type NotificationTone = "info" | "success" | "error";

type NotificationInput = {
    message: string;
    tone?: NotificationTone;
    duration?: number;
};

type Notification = {
    id: number;
    message: string;
    tone: NotificationTone;
    duration: number;
};

type NotificationContextValue = {
    notify: (input: NotificationInput) => void;
    dismiss: (id: number) => void;
    systemPermission: SystemNotificationPermission;
    enableSystemNotifications: () => Promise<void>;
};

const NotificationContext = createContext<NotificationContextValue | null>(
    null,
);
let nextNotificationId = 0;

type SystemNotificationPermission = NotificationPermission | "unsupported";

export function NotificationProvider({
                                         children,
                                         dismissLabel,
                                     }: {
    children: ReactNode;
    dismissLabel: string;
}) {
    const [notifications, setNotifications] = useState<Notification[]>([]);
    const [systemPermission, setSystemPermission] =
        useState<SystemNotificationPermission>(getSystemNotificationPermission);
    const timers = useRef(new Map<number, number>());

    const dismiss = useCallback((id: number) => {
        const timer = timers.current.get(id);
        if (timer !== undefined) {
            window.clearTimeout(timer);
            timers.current.delete(id);
        }
        setNotifications((current) =>
            current.filter((notification) => notification.id !== id),
        );
    }, []);

    const notify = useCallback((input: NotificationInput) => {
        const id = ++nextNotificationId;
        const notification: Notification = {
            id,
            message: input.message,
            tone: input.tone ?? "info",
            duration: input.duration ?? 5000,
        };
        setNotifications((current) => [...current, notification].slice(-4));
        if (
            typeof window !== "undefined" &&
            "Notification" in window &&
            window.Notification.permission === "granted"
        ) {
            try {
                const systemNotification = new window.Notification("Host Bridge", {
                    body: notification.message,
                    tag: `host-bridge-${notification.tone}-${id}`,
                });
                systemNotification.onclick = () => {
                    window.focus();
                    systemNotification.close();
                };
            } catch {
                // Keep the in-app notification when the platform notification fails.
            }
        }
        if (notification.duration && notification.duration > 0) {
            const timer = window.setTimeout(() => {
                timers.current.delete(id);
                setNotifications((current) =>
                    current.filter((value) => value.id !== id),
                );
            }, notification.duration);
            timers.current.set(id, timer);
        }
    }, []);

    const enableSystemNotifications = useCallback(async () => {
        if (typeof window === "undefined" || !("Notification" in window)) {
            setSystemPermission("unsupported");
            return;
        }
        try {
            const permission = await window.Notification.requestPermission();
            setSystemPermission(permission);
        } catch {
            setSystemPermission("denied");
        }
    }, []);

    useEffect(
        () => () => {
            for (const timer of timers.current.values()) {
                window.clearTimeout(timer);
            }
            timers.current.clear();
        },
        [],
    );

    return (
        <NotificationContext.Provider
            value={{
                notify,
                dismiss,
                systemPermission,
                enableSystemNotifications,
            }}
        >
            {children}
            <div
                aria-live="polite"
                className="pointer-events-none fixed inset-x-4 top-4 z-50 flex flex-col items-end gap-2 sm:left-auto sm:w-[min(24rem,calc(100vw-2rem))]"
            >
                {notifications.map((notification) => {
                    const Icon = notificationIcon(notification.tone);
                    return (
                        <Alert
                            key={notification.id}
                            variant={
                                notification.tone === "error" ? "destructive" : "default"
                            }
                            className={cn(
                                "pointer-events-auto bg-background/95 backdrop-blur-sm animate-in fade-in-0 slide-in-from-top-2 duration-200",
                                notification.tone === "info" &&
                                "border-primary/30 bg-primary/5 text-foreground",
                                notification.tone === "success" &&
                                "border-emerald-200 bg-emerald-50 text-emerald-900",
                            )}
                        >
                            <Icon className="size-4"/>
                            <AlertDescription className="pr-5 text-inherit">
                                {notification.message}
                            </AlertDescription>
                            <Button
                                variant="ghost"
                                size="icon-xs"
                                className="absolute right-2 top-2 text-current hover:bg-black/5"
                                onClick={() => dismiss(notification.id)}
                                aria-label={dismissLabel}
                                title={dismissLabel}
                            >
                                <X className="size-3.5"/>
                            </Button>
                        </Alert>
                    );
                })}
            </div>
        </NotificationContext.Provider>
    );
}

function getSystemNotificationPermission(): SystemNotificationPermission {
    if (typeof window === "undefined" || !("Notification" in window)) {
        return "unsupported";
    }
    return window.Notification.permission;
}

export function useNotifications(): NotificationContextValue {
    const context = useContext(NotificationContext);
    if (!context) {
        throw new Error(
            "useNotifications must be used within NotificationProvider",
        );
    }
    return context;
}

function notificationIcon(tone: NotificationTone) {
    if (tone === "success") return CheckCircle2;
    if (tone === "error") return CircleAlert;
    return Info;
}
