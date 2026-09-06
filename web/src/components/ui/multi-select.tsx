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

import {ChevronsUpDown} from "lucide-react";
import {Button} from "@/components/ui/button";
import {
    DropdownMenu,
    DropdownMenuCheckboxItem,
    DropdownMenuContent,
    DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {cn} from "@/lib/utils";

export function MultiSelect({
                                options,
                                value,
                                onChange,
                                placeholder,
                                allLabel,
                                className,
                            }: {
    options: Array<{ value: string; label: string }>;
    value: string[];
    onChange: (value: string[]) => void;
    placeholder: string;
    allLabel: string;
    className?: string;
}) {
    const selected = new Set(value);
    const selectedLabels = options
        .filter((option) => selected.has(option.value))
        .map((option) => option.label);
    const label =
        selectedLabels.length === 0
            ? allLabel
            : selectedLabels.join(", ") || placeholder;

    return (
        <DropdownMenu>
            <DropdownMenuTrigger asChild>
                <Button
                    type="button"
                    variant="outline"
                    className={cn(
                        "w-full justify-between bg-background text-left",
                        className,
                    )}
                    aria-label={placeholder}
                >
                    <span className="min-w-0 truncate">{label}</span>
                    <ChevronsUpDown className="size-4 shrink-0 text-muted-foreground"/>
                </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent
                align="start"
                className="w-[var(--radix-dropdown-menu-trigger-width)] min-w-56"
            >
                {options.map((option) => (
                    <DropdownMenuCheckboxItem
                        key={option.value}
                        checked={selected.has(option.value)}
                        onSelect={(event) => event.preventDefault()}
                        onCheckedChange={(checked) => {
                            const next = new Set(value);
                            if (checked) next.add(option.value);
                            else next.delete(option.value);
                            onChange(
                                options
                                    .filter((item) => next.has(item.value))
                                    .map((item) => item.value),
                            );
                        }}
                    >
                        <span className="min-w-0 truncate">{option.label}</span>
                    </DropdownMenuCheckboxItem>
                ))}
            </DropdownMenuContent>
        </DropdownMenu>
    );
}
