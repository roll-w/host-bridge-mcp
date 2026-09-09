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

function Kbd({className, ...props}: React.ComponentProps<"kbd">) {
    return (
        <kbd
            data-slot="kbd"
            className={cn(
                "inline-flex min-w-5 items-center justify-center rounded border border-border/80 bg-muted px-1.5 py-0.5 font-mono text-[0.68rem] font-semibold leading-none text-muted-foreground shadow-sm",
                className,
            )}
            {...props}
        />
    );
}

export {Kbd};
