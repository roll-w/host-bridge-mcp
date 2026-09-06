import { useEffect, useRef, useState } from "react";
import { ChevronRight, CircleCheck, CircleX, LoaderCircle } from "lucide-react";
import { apiRequest, parseEvent } from "@/api";
import { type MessageKey } from "@/i18n";
import type { HistoryRecord } from "@/types";
import { EmptyState, SectionHeading } from "@/components/layout";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { cn } from "@/lib/utils";

type StreamState = "waiting" | "connecting" | "live" | "reconnecting";

type StatusEvent = {
  type: "status";
  state: HistoryRecord["state"];
  message: string | null;
};

type OutputEvent = {
  type: "output";
  text: string;
};

type ExitEvent = {
  type: "exit";
  code: number;
  success: boolean;
  timedOut: boolean;
};

type ExecutionSummaryMap = Record<string, HistoryRecord>;

export function RunningExecutionsPanel({
  executionIds,
  t,
}: {
  executionIds: string[];
  t: (key: MessageKey) => string;
}) {
  const [selectedId, setSelectedId] = useState<string | null>(
    executionIds[0] ?? null,
  );
  const [summaries, setSummaries] = useState<ExecutionSummaryMap>({});

  useEffect(() => {
    if (selectedId && executionIds.includes(selectedId)) return;
    setSelectedId(executionIds[0] ?? null);
  }, [executionIds, selectedId]);

  useEffect(() => {
    let active = true;

    if (executionIds.length === 0) {
      setSummaries({});
      return () => undefined;
    }

    const loadSummaries = async () => {
      const results = await Promise.all(
        executionIds.map(async (executionId) => {
          try {
            const record = await apiRequest<HistoryRecord>(
              `/history/${encodeURIComponent(executionId)}`,
            );
            return [executionId, record] as const;
          } catch {
            return null;
          }
        }),
      );

      if (!active) return;

      const next: ExecutionSummaryMap = {};
      for (const result of results) {
        if (result) next[result[0]] = result[1];
      }
      setSummaries(next);
    };

    void loadSummaries();
    const timer = window.setInterval(() => void loadSummaries(), 1_000);
    return () => {
      active = false;
      window.clearInterval(timer);
    };
  }, [executionIds]);

  return (
    <div className="space-y-4 xl:sticky xl:top-6">
      <SectionHeading
        title={t("runningExecutions")}
        action={
          executionIds.length > 0 ? (
            <span className="text-xs tabular-nums text-muted-foreground">
              {executionIds.length}
            </span>
          ) : undefined
        }
      />
      {executionIds.length === 0 ? (
        <EmptyState className="min-h-44 px-3 py-6 text-xs">
          {t("noRunningExecutions")}
        </EmptyState>
      ) : (
        <div className="overflow-hidden rounded-lg border border-border bg-card">
          <ScrollArea className="max-h-64">
            <div className="space-y-1 p-2">
              {executionIds.map((executionId) => {
                const record = summaries[executionId];
                const selected = executionId === selectedId;
                return (
                  <Button
                    key={executionId}
                    type="button"
                    variant="ghost"
                    onClick={() => setSelectedId(executionId)}
                    className={cn(
                      "h-auto w-full justify-start gap-3 rounded-md px-3 py-3 text-left",
                      selected && "bg-accent/70 hover:bg-accent",
                    )}
                  >
                    <ExecutionStateIcon state={record?.state ?? "running"} />
                    <span className="min-w-0 flex-1">
                      <span className="block truncate font-mono text-xs text-foreground">
                        {record?.commandLine ?? executionId}
                      </span>
                      <span className="mt-1 block truncate text-[11px] text-muted-foreground">
                        {record?.server ?? executionId}
                      </span>
                    </span>
                    <ChevronRight className="size-4 text-muted-foreground" />
                  </Button>
                );
              })}
            </div>
          </ScrollArea>
          {selectedId && (
            <div className="border-t border-border/70">
              <RunningExecutionDetail executionId={selectedId} t={t} />
            </div>
          )}
        </div>
      )}
    </div>
  );
}

export function RunningExecutionPanel({
  executionId,
  t,
}: {
  executionId: string | null;
  t: (key: MessageKey) => string;
}) {
  return (
    <div className="overflow-hidden rounded-lg border border-border bg-card">
      <RunningExecutionDetail executionId={executionId} t={t} />
    </div>
  );
}

function RunningExecutionDetail({
  executionId,
  t,
}: {
  executionId: string | null;
  t: (key: MessageKey) => string;
}) {
  const [record, setRecord] = useState<HistoryRecord | null>(null);
  const [output, setOutput] = useState("");
  const [streamState, setStreamState] = useState<StreamState>("waiting");
  const [statusMessage, setStatusMessage] = useState<string | null>(null);
  const outputRef = useRef("");

  useEffect(() => {
    let active = true;
    let source: EventSource | null = null;
    let retryTimer: number | undefined;
    let outputPollTimer: number | undefined;

    setRecord(null);
    setOutput("");
    outputRef.current = "";
    setStatusMessage(null);
    setStreamState("waiting");

    if (!executionId) {
      return () => undefined;
    }

    const setLatestOutput = (nextOutput: string) => {
      if (!active || nextOutput.length < outputRef.current.length) return;
      outputRef.current = nextOutput;
      setOutput(nextOutput);
    };

    const refreshOutput = async () => {
      try {
        const value = await apiRequest<{ output: string }>(
          `/history/${encodeURIComponent(executionId)}/output`,
        );
        setLatestOutput(value.output);
      } catch {
        // The history record may be finalized a moment after the stream closes.
      }
    };

    const loadExecution = async () => {
      try {
        const nextRecord = await apiRequest<HistoryRecord>(
          `/history/${encodeURIComponent(executionId)}`,
        );
        if (!active) return;

        const outputResponse = await apiRequest<{ output: string }>(
          `/history/${encodeURIComponent(executionId)}/output`,
        );
        if (!active) return;

        setRecord(nextRecord);
        setLatestOutput(outputResponse.output);
        setStreamState(nextRecord.state === "running" ? "connecting" : "live");

        if (nextRecord.state === "running") {
          const stopOutputPolling = () => {
            if (outputPollTimer !== undefined) {
              window.clearInterval(outputPollTimer);
              outputPollTimer = undefined;
            }
          };
          source = new EventSource(
            `/api/v1/executions/${encodeURIComponent(executionId)}/stream`,
          );
          source.addEventListener("open", () => {
            if (active) setStreamState("live");
          });
          source.addEventListener("status", (event) => {
            const value = parseEvent<StatusEvent>(event);
            if (!active || !value) return;
            setStatusMessage(value.message);
            setRecord((current) =>
              current ? { ...current, state: value.state } : current,
            );
            if (value.state !== "running") {
              stopOutputPolling();
              source?.close();
              void refreshOutput();
            }
          });
          source.addEventListener("output", (event) => {
            const value = parseEvent<OutputEvent>(event);
            if (active && value) {
              setOutput((current) => {
                const nextOutput = current + value.text;
                outputRef.current = nextOutput;
                return nextOutput;
              });
            }
          });
          source.addEventListener("exit", (event) => {
            const value = parseEvent<ExitEvent>(event);
            if (!active || !value) return;
            stopOutputPolling();
            setRecord((current) =>
              current
                ? {
                    ...current,
                    state: value.success ? "completed" : "failed",
                    exitCode: value.code,
                    success: value.success,
                    timedOut: value.timedOut,
                  }
                : current,
            );
            void refreshOutput();
          });
          source.addEventListener("error", (event) => {
            const value = parseEvent<{ message: string }>(event);
            if (active && value) setStatusMessage(value.message);
          });
          source.onerror = () => {
            if (active) setStreamState("reconnecting");
          };
          outputPollTimer = window.setInterval(() => {
            void refreshOutput();
          }, 500);
        }
      } catch {
        if (active) {
          setStreamState("waiting");
          retryTimer = window.setTimeout(loadExecution, 500);
        }
      }
    };

    void loadExecution();

    return () => {
      active = false;
      source?.close();
      if (retryTimer !== undefined) window.clearTimeout(retryTimer);
      if (outputPollTimer !== undefined) window.clearInterval(outputPollTimer);
    };
  }, [executionId]);

  return (
    <div className="space-y-4 p-5">
      {!executionId ? (
        <EmptyState className="min-h-44 px-3 py-6 text-xs">
          {t("noRunningExecutions")}
        </EmptyState>
      ) : !record ? (
        <div className="flex min-h-44 items-center justify-center gap-2 text-sm text-muted-foreground">
          <LoaderCircle className="size-4 animate-spin" />
          {t("waitingForExecution")}
        </div>
      ) : (
        <>
          <div className="flex items-start justify-between gap-3">
            <div className="min-w-0 space-y-1">
              <p className="break-words font-mono text-sm leading-6 text-foreground">
                {record.commandLine}
              </p>
              <p className="truncate text-xs text-muted-foreground">
                {record.server}
              </p>
            </div>
            <div className="flex shrink-0 flex-col items-end gap-2 text-xs">
              {record.state === "running" && (
                <StreamIndicator state={streamState} t={t} />
              )}
              <ExecutionState state={record.state} t={t} />
            </div>
          </div>
          {statusMessage && record.state === "running" && (
            <p className="text-xs leading-5 text-muted-foreground">
              {statusMessage}
            </p>
          )}
          <ScrollArea className="h-[min(46vh,32rem)] min-h-56 rounded-md bg-muted/35">
            <pre className="whitespace-pre-wrap break-words p-4 font-mono text-xs leading-5 text-foreground/85">
              {output || " "}
            </pre>
          </ScrollArea>
        </>
      )}
    </div>
  );
}

function StreamIndicator({
  state,
  t,
}: {
  state: StreamState;
  t: (key: MessageKey) => string;
}) {
  const label =
    state === "reconnecting"
      ? t("streamReconnecting")
      : state === "live"
        ? t("streamLive")
        : state === "connecting"
          ? t("streamConnecting")
          : t("waitingForExecution");
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1.5 text-[11px] font-medium",
        state === "live" ? "text-emerald-700" : "text-muted-foreground",
      )}
    >
      <span
        className={cn(
          "size-1.5 rounded-full",
          state === "live" ? "bg-emerald-600" : "bg-muted-foreground/60",
        )}
      />
      {label}
    </span>
  );
}

function ExecutionStateIcon({ state }: { state: HistoryRecord["state"] }) {
  const Icon =
    state === "running"
      ? LoaderCircle
      : state === "completed"
        ? CircleCheck
        : CircleX;
  return (
    <Icon
      className={cn(
        "size-4 shrink-0",
        state === "running" && "animate-spin text-primary",
        state === "completed" && "text-emerald-700",
        state === "failed" && "text-destructive",
      )}
    />
  );
}

function ExecutionState({
  state,
  t,
}: {
  state: HistoryRecord["state"];
  t: (key: MessageKey) => string;
}) {
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1",
        state === "completed"
          ? "text-emerald-700"
          : state === "failed"
            ? "text-destructive"
            : "text-amber-700",
      )}
    >
      <ExecutionStateIcon state={state} />
      {state === "running"
        ? t("stateRunning")
        : state === "completed"
          ? t("stateCompleted")
          : t("stateFailed")}
    </span>
  );
}
