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

"use client";

import * as React from "react";
import {cn} from "@/lib/utils";
import {ScrollArea as ScrollAreaPrimitive} from "radix-ui";

function ScrollArea({
                        className,
                        children,
                        ...props
                    }: React.ComponentProps<typeof ScrollAreaPrimitive.Root>) {
    return (
        <ScrollAreaPrimitive.Root
            data-slot="scroll-area"
            className={cn("relative", className)}
            {...props}
        >
            <ScrollAreaPrimitive.Viewport
                data-slot="scroll-area-viewport"
                className="size-full rounded-[inherit] transition-[color,box-shadow] outline-none focus-visible:ring-[3px] focus-visible:ring-oklch(0.708 0 0)/50 focus-visible:outline-1 dark:focus-visible:ring-oklch(0.556 0 0)/50"
            >
                {children}
            </ScrollAreaPrimitive.Viewport>
            <ScrollBar/>
            <ScrollAreaPrimitive.Corner/>
        </ScrollAreaPrimitive.Root>
    );
}

function ScrollBar({
                       className,
                       orientation = "vertical",
                       ...props
                   }: React.ComponentProps<typeof ScrollAreaPrimitive.ScrollAreaScrollbar>) {
    return (
        <ScrollAreaPrimitive.ScrollAreaScrollbar
            data-slot="scroll-area-scrollbar"
            data-orientation={orientation}
            orientation={orientation}
            className={cn(
                "flex touch-none p-px transition-colors select-none data-horizontal:h-2.5 data-horizontal:flex-col data-horizontal:border-t data-horizontal:border-t-transparent data-vertical:h-full data-vertical:w-2.5 data-vertical:border-l data-vertical:border-l-transparent",
                className,
            )}
            {...props}
        >
            <ScrollAreaPrimitive.ScrollAreaThumb
                data-slot="scroll-area-thumb"
                className="relative flex-1 rounded-full bg-oklch(0.922 0 0) dark:bg-oklch(1 0 0 / 10%)"
            />
        </ScrollAreaPrimitive.ScrollAreaScrollbar>
    );
}

export {ScrollArea, ScrollBar};
