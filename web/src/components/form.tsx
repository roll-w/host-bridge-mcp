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

import type {ComponentProps} from "react";
import {Input} from "@/components/ui/input";
import {Label} from "@/components/ui/label";
import {Select, SelectContent, SelectItem, SelectTrigger, SelectValue,} from "@/components/ui/select";
import {Switch} from "@/components/ui/switch";
import {cn} from "@/lib/utils";

export function Field({
                          label,
                          value,
                          onChange,
                          mono = false,
                          placeholder,
                          description,
                          type,
                      }: {
    label: string;
    value: string;
    onChange: (value: string) => void;
    mono?: boolean;
    placeholder?: string;
    description?: string;
    type?: ComponentProps<typeof Input>["type"];
}) {
    return (
        <div className="space-y-1.5">
            <Label>{label}</Label>
            <Input
                type={type}
                value={value}
                placeholder={placeholder}
                onChange={(event) => onChange(event.target.value)}
                className={cn(mono && "font-mono text-xs")}
            />
            {description && (
                <p className="text-xs leading-5 text-muted-foreground">{description}</p>
            )}
        </div>
    );
}

export function NumberField({
                                label,
                                value,
                                onChange,
                                description,
                                min = 1,
                            }: {
    label: string;
    value: number;
    onChange: (value: number) => void;
    description?: string;
    min?: number;
}) {
    return (
        <div className="space-y-1.5">
            <Label>{label}</Label>
            <Input
                type="number"
                min={min}
                value={value}
                onChange={(event) => onChange(Number(event.target.value))}
            />
            {description && (
                <p className="text-xs leading-5 text-muted-foreground">{description}</p>
            )}
        </div>
    );
}

export function SelectField({
                                label,
                                value,
                                options,
                                onChange,
                                description,
                            }: {
    label: string;
    value: string;
    options: Array<{ value: string; label: string }>;
    onChange: (value: string) => void;
    description?: string;
}) {
    return (
        <div className="space-y-1.5">
            <Label>{label}</Label>
            <Select value={value} onValueChange={onChange}>
                <SelectTrigger className="w-full bg-background">
                    <SelectValue/>
                </SelectTrigger>
                <SelectContent>
                    {options.map((option) => (
                        <SelectItem key={option.value} value={option.value}>
                            {option.label}
                        </SelectItem>
                    ))}
                </SelectContent>
            </Select>
            {description && (
                <p className="text-xs leading-5 text-muted-foreground">{description}</p>
            )}
        </div>
    );
}

export function ToggleField({
                                label,
                                value,
                                onChange,
                                description,
                            }: {
    label: string;
    value: boolean;
    onChange: (value: boolean) => void;
    description?: string;
}) {
    return (
        <div className="flex items-start justify-between gap-4 p-1">
            <div>
                <div className="text-sm font-medium">{label}</div>
                {description && (
                    <p className="mt-1 text-xs leading-5 text-muted-foreground">
                        {description}
                    </p>
                )}
            </div>
            <Switch checked={value} onCheckedChange={onChange}/>
        </div>
    );
}
