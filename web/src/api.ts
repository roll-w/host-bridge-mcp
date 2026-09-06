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

export interface ApiEnvelope<T> {
    status: { code: number; message: string };
    data: T;
}

export class ApiRequestError extends Error {
    constructor(
        message: string,
        readonly code: number,
        readonly httpStatus: number,
    ) {
        super(message);
    }
}

export async function apiRequest<T>(
    path: string,
    init?: RequestInit,
): Promise<T> {
    const response = await fetch(`/api/v1${path}`, {
        credentials: "include",
        ...init,
        headers: {
            "Content-Type": "application/json",
            ...(init?.headers ?? {}),
        },
    });

    let envelope: ApiEnvelope<T>;
    try {
        envelope = (await response.json()) as ApiEnvelope<T>;
    } catch {
        throw new ApiRequestError(
            response.statusText || "Request failed",
            -1,
            response.status,
        );
    }

    if (!response.ok || envelope.status.code !== 0) {
        throw new ApiRequestError(
            envelope.status.message,
            envelope.status.code,
            response.status,
        );
    }
    return envelope.data;
}

export function jsonBody(value: unknown): RequestInit {
    return {method: "POST", body: JSON.stringify(value)};
}

export function parseEvent<T>(event: Event): T | null {
    try {
        const payload = JSON.parse((event as MessageEvent).data) as ApiEnvelope<T>;
        return payload.status ? payload.data : (payload as unknown as T);
    } catch {
        return null;
    }
}
