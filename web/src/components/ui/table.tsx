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

function Table({className, ...props}: React.ComponentProps<"table">) {
    return (
        <div
            data-slot="table-container"
            className="relative w-full overflow-x-auto"
        >
            <table
                data-slot="table"
                className={cn("w-full caption-bottom text-sm", className)}
                {...props}
            />
        </div>
    );
}

function TableHeader({className, ...props}: React.ComponentProps<"thead">) {
    return (
        <thead
            data-slot="table-header"
            className={cn("[&_tr]:border-b", className)}
            {...props}
        />
    );
}

function TableBody({className, ...props}: React.ComponentProps<"tbody">) {
    return (
        <tbody
            data-slot="table-body"
            className={cn("[&_tr:last-child]:border-0", className)}
            {...props}
        />
    );
}

function TableFooter({className, ...props}: React.ComponentProps<"tfoot">) {
    return (
        <tfoot
            data-slot="table-footer"
            className={cn(
                "border-t bg-oklch(0.97 0 0)/50 font-medium [&>tr]:last:border-b-0 dark:bg-oklch(0.269 0 0)/50",
                className,
            )}
            {...props}
        />
    );
}

function TableRow({className, ...props}: React.ComponentProps<"tr">) {
    return (
        <tr
            data-slot="table-row"
            className={cn(
                "border-b transition-colors hover:bg-oklch(0.97 0 0)/50 has-aria-expanded:bg-oklch(0.97 0 0)/50 data-[state=selected]:bg-oklch(0.97 0 0) dark:hover:bg-oklch(0.269 0 0)/50 dark:has-aria-expanded:bg-oklch(0.269 0 0)/50 dark:data-[state=selected]:bg-oklch(0.269 0 0)",
                className,
            )}
            {...props}
        />
    );
}

function TableHead({className, ...props}: React.ComponentProps<"th">) {
    return (
        <th
            data-slot="table-head"
            className={cn(
                "h-10 px-2 text-left align-middle font-medium whitespace-nowrap text-oklch(0.145 0 0) [&:has([role=checkbox])]:pr-0 dark:text-oklch(0.985 0 0)",
                className,
            )}
            {...props}
        />
    );
}

function TableCell({className, ...props}: React.ComponentProps<"td">) {
    return (
        <td
            data-slot="table-cell"
            className={cn(
                "p-2 align-middle whitespace-nowrap [&:has([role=checkbox])]:pr-0",
                className,
            )}
            {...props}
        />
    );
}

function TableCaption({
                          className,
                          ...props
                      }: React.ComponentProps<"caption">) {
    return (
        <caption
            data-slot="table-caption"
            className={cn(
                "mt-4 text-sm text-oklch(0.556 0 0) dark:text-oklch(0.708 0 0)",
                className,
            )}
            {...props}
        />
    );
}

export {
    Table,
    TableHeader,
    TableBody,
    TableFooter,
    TableHead,
    TableRow,
    TableCell,
    TableCaption,
};
