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

import * as React from "react";
import {cn} from "@/lib/utils";
import {Switch as SwitchPrimitive} from "radix-ui";

function Switch({
                    className,
                    size = "default",
                    ...props
                }: React.ComponentProps<typeof SwitchPrimitive.Root> & {
    size?: "sm" | "default";
}) {
    return (
        <SwitchPrimitive.Root
            data-slot="switch"
            data-size={size}
            className={cn(
                "peer relative inline-flex shrink-0 items-center rounded-full border border-input border-transparent bg-muted outline-none transition-colors focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/30 disabled:cursor-not-allowed disabled:opacity-60 data-[state=checked]:bg-primary data-[state=unchecked]:bg-muted data-[size=default]:h-5 data-[size=default]:w-9 data-[size=sm]:h-4 data-[size=sm]:w-7",
                className,
            )}
            {...props}
        >
            <SwitchPrimitive.Thumb
                className="pointer-events-none block size-4 rounded-full bg-background shadow-none transition-transform data-[state=checked]:translate-x-4 data-[state=unchecked]:translate-x-0 data-[size=sm]:size-3 data-[size=sm]:data-[state=checked]:translate-x-3"/>
        </SwitchPrimitive.Root>
    );
}

export {Switch};
