import { useEffect, useMemo, useState } from "react";
import { Link, useParams } from "react-router-dom";
import { Bot, FileDiff, Square } from "lucide-react";
import { EmptyState } from "@/components/empty-states/EmptyState";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { MonacoDiffViewer } from "@/components/diff/MonacoDiffViewer";
import { useAgentStore } from "@/stores/useAgentStore";
import { useSettingsStore } from "@/stores/useSettingsStore";
import {
  getAgentRun,
  getRunDiff,
  listActivityEvents,
  listAgentRuns,
  listAgents,
  listToolCalls,
  startAgentRun,
  stopAgentRun,
} from "@/lib/tauri";
import { onForgeEvent } from "@/lib/events";
import { toastError } from "@/stores/useToastStore";
import { cn } from "@/lib/utils";
import type { Agent, AgentRunFileDiffDto, AgentRunStatus, ToolCallStatus } from "@/types/db";

function errorMessage(err: unknown): string {
  if (typeof err === "string") return err;
  if (err instanceof Error) return err.message;
  return String(err);
}

const RUN_STATUS_BADGE: Record<AgentRunStatus, { label: string; variant: "default" | "success" | "destructive" | "warning" | "secondary" }> = {
  queued: { label: "Queued", variant: "secondary" },
  running: { label: "Running", variant: "default" },
  completed: { label: "Completed", variant: "success" },
  failed: { label: "Failed", variant: "destructive" },
  stopped: { label: "Stopped", variant: "warning" },
};

const TOOL_CALL_BADGE: Record<ToolCallStatus, { label: string; variant: "default" | "success" | "destructive" }> = {
  running: { label: "running", variant: "default" },
  success: { label: "success", variant: "success" },
  error: { label: "error", variant: "destructive" },
};

function formatActivitySummary(payloadJson: string): string {
  try {
    const parsed = JSON.parse(payloadJson) as Record<string, unknown>;
    if (typeof parsed.text === "string" && parsed.text.trim().length > 0) return parsed.text;
    if (typeof parsed.summary === "string") return parsed.summary;
    if (typeof parsed.errorMessage === "string" && parsed.errorMessage) return parsed.errorMessage;
    if (typeof parsed.taskPrompt === "string") return `Task: ${parsed.taskPrompt}`;
    return payloadJson;
  } catch {
    return payloadJson;
  }
}

/** Diff panel: file list + `MonacoDiffViewer`, mirroring `ChangesTab.tsx`'s layout. */
function RunDiffPanel({ agentRunId }: { agentRunId: string }) {
  const [files, setFiles] = useState<AgentRunFileDiffDto[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [selectedPath, setSelectedPath] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setFiles(null);
    setError(null);
    setSelectedPath(null);
    getRunDiff(agentRunId)
      .then((result) => {
        if (cancelled) return;
        setFiles(result);
        setSelectedPath(result[0]?.path ?? null);
      })
      .catch((err) => {
        if (!cancelled) setError(errorMessage(err));
      });
    return () => {
      cancelled = true;
    };
  }, [agentRunId]);

  const selected = files?.find((f) => f.path === selectedPath) ?? null;

  if (error) {
    return (
      <div className="rounded-md border border-destructive/40 bg-destructive/10 px-2 py-1.5 text-xs text-destructive">
        {error}
      </div>
    );
  }
  if (!files) {
    return <p className="text-xs text-muted-foreground">Loading diff…</p>;
  }
  if (files.length === 0) {
    return <EmptyState icon={FileDiff} title="No changes" description="This run's workspace has no uncommitted changes." />;
  }

  return (
    <div className="flex flex-1 gap-4 overflow-hidden">
      <div className="flex w-56 shrink-0 flex-col gap-0.5 overflow-y-auto">
        {files.map((file) => (
          <button
            key={file.path}
            type="button"
            title={file.path}
            onClick={() => setSelectedPath(file.path)}
            className={cn(
              "truncate rounded px-2 py-1 text-left text-xs text-foreground hover:bg-surface-hover",
              selectedPath === file.path && "bg-surface-hover",
            )}
          >
            {file.path}
          </button>
        ))}
      </div>
      <MonacoDiffViewer
        original={selected?.original}
        modified={selected?.modified}
        path={selected?.path}
        className="flex-1"
      />
    </div>
  );
}

/**
 * Real Agent Detail page: shows the current run for `agentId` (its task
 * prompt, live status, activity feed, tool call cards, and a diff of what
 * changed), with a working Start (for a `queued` run) and Stop (for a
 * `running` one) button. All live data comes from `agent-run:*` events via
 * `useAgentStore` — nothing here is simulated.
 */
export function AgentDetail() {
  const { projectId, agentId } = useParams<{ projectId: string; agentId: string }>();

  const [agent, setAgent] = useState<Agent | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [currentRunId, setCurrentRunId] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [isStarting, setIsStarting] = useState(false);
  const [isStopping, setIsStopping] = useState(false);

  const hasApiKey = useSettingsStore((s) => s.hasApiKey);
  const checkApiKey = useSettingsStore((s) => s.checkApiKey);

  const loadRun = useAgentStore((s) => s.loadRun);
  const setRun = useAgentStore((s) => s.setRun);
  const appendActivity = useAgentStore((s) => s.appendActivity);
  const upsertToolCall = useAgentStore((s) => s.upsertToolCall);
  const appendMessageDelta = useAgentStore((s) => s.appendMessageDelta);
  const runState = useAgentStore((s) => (currentRunId ? s.runsById[currentRunId] : undefined));

  useEffect(() => {
    void checkApiKey();
  }, [checkApiKey]);

  // Initial load: the agent row, its most recent run, and that run's full
  // tool-call/activity history so far.
  useEffect(() => {
    if (!projectId || !agentId) return;
    let cancelled = false;
    setIsLoading(true);
    setLoadError(null);
    setCurrentRunId(null);

    (async () => {
      const agents = await listAgents(projectId);
      const found = agents.find((a) => a.id === agentId) ?? null;
      if (cancelled) return;
      setAgent(found);
      if (!found) return;

      const runs = await listAgentRuns(agentId);
      const latest = runs[0] ?? null;
      if (cancelled || !latest) return;

      const [toolCalls, activity] = await Promise.all([listToolCalls(latest.id), listActivityEvents(latest.id)]);
      if (cancelled) return;
      loadRun(latest.id, latest, activity, toolCalls);
      setCurrentRunId(latest.id);
    })()
      .catch((err) => {
        if (!cancelled) setLoadError(errorMessage(err));
      })
      .finally(() => {
        if (!cancelled) setIsLoading(false);
      });

    return () => {
      cancelled = true;
    };
  }, [projectId, agentId, loadRun]);

  // Live updates for the current run, once known.
  useEffect(() => {
    if (!currentRunId) return;
    let cancelled = false;
    const unlisten: Array<() => void> = [];

    void (async () => {
      unlisten.push(
        await onForgeEvent("agent-run:status-changed", (payload) => {
          if (cancelled || payload.agentRunId !== currentRunId) return;
          // The event payload only carries the new status; refetch the full
          // row (token counts, stop reason, timestamps) rather than
          // synthesizing a partial one. This runs in the background (not
          // from a user action), so a failure has no natural inline banner
          // to land in — surface it via toast instead of swallowing it.
          getAgentRun(currentRunId)
            .then((run) => {
              if (!cancelled) setRun(currentRunId, run);
            })
            .catch((err) => {
              if (!cancelled) toastError("Couldn't refresh run status", errorMessage(err));
            });
        }),
      );
      unlisten.push(
        await onForgeEvent("agent-run:activity", (payload) => {
          if (!cancelled && payload.agentRunId === currentRunId) appendActivity(currentRunId, payload.event);
        }),
      );
      unlisten.push(
        await onForgeEvent("agent-run:tool-call-updated", (payload) => {
          if (!cancelled && payload.agentRunId === currentRunId) upsertToolCall(currentRunId, payload.toolCall);
        }),
      );
      unlisten.push(
        await onForgeEvent("agent-run:message-delta", (payload) => {
          if (!cancelled && payload.agentRunId === currentRunId) appendMessageDelta(currentRunId, payload.text);
        }),
      );
    })();

    return () => {
      cancelled = true;
      unlisten.forEach((fn) => fn());
    };
  }, [currentRunId, setRun, appendActivity, upsertToolCall, appendMessageDelta]);

  const run = runState?.run ?? null;
  const activity = useMemo(() => runState?.activity ?? [], [runState]);
  const toolCalls = useMemo(
    () => (runState ? runState.toolCallOrder.map((id) => runState.toolCallsById[id]) : []),
    [runState],
  );
  const streamingText = runState?.streamingText ?? "";

  const handleStart = async () => {
    if (!run) return;
    setActionError(null);
    setIsStarting(true);
    try {
      await startAgentRun(run.id);
    } catch (err) {
      setActionError(errorMessage(err));
    } finally {
      setIsStarting(false);
    }
  };

  const handleStop = async () => {
    if (!run) return;
    setActionError(null);
    setIsStopping(true);
    try {
      await stopAgentRun(run.id);
    } catch (err) {
      setActionError(errorMessage(err));
    } finally {
      setIsStopping(false);
    }
  };

  if (isLoading) {
    return <p className="p-4 text-xs text-muted-foreground">Loading…</p>;
  }

  if (loadError || !agent) {
    return (
      <EmptyState
        icon={Bot}
        title="Agent not found"
        description={loadError ?? `No agent "${agentId}" found for this project.`}
      />
    );
  }

  if (!run) {
    return (
      <div className="flex flex-1 flex-col items-center justify-center gap-3">
        <EmptyState
          icon={Bot}
          title="No run yet"
          description={`"${agent.name}" has no runs. Go to Agents and click Start to create one.`}
        />
        <Button asChild variant="secondary" size="sm">
          <Link to="/agents">Go to Agents</Link>
        </Button>
      </div>
    );
  }

  const statusBadge = RUN_STATUS_BADGE[run.status];
  const isRunning = run.status === "running";
  const isQueued = run.status === "queued";
  const isTerminal = run.status === "completed" || run.status === "failed" || run.status === "stopped";

  return (
    <div className="flex flex-1 flex-col gap-4 overflow-hidden">
      <div className="flex items-start justify-between gap-3">
        <div className="flex flex-col gap-1">
          <div className="flex items-center gap-2">
            <h1 className="text-lg font-semibold text-foreground">{agent.name}</h1>
            <Badge variant={statusBadge.variant}>{statusBadge.label}</Badge>
          </div>
          <p className="max-w-2xl text-xs text-muted-foreground">{run.taskPrompt}</p>
          <p className="text-[11px] text-subtle-foreground">
            model {run.modelId} · iteration {run.iterationCount} · {run.totalInputTokens.toLocaleString()} in /{" "}
            {run.totalOutputTokens.toLocaleString()} out tokens
            {run.stopReason && ` · stop reason: ${run.stopReason}`}
          </p>
          {run.errorMessage && <p className="max-w-2xl text-xs text-destructive">{run.errorMessage}</p>}
        </div>

        <div className="flex shrink-0 flex-col items-end gap-1">
          {isQueued && !hasApiKey && (
            <p className="max-w-xs text-right text-[11px] text-destructive">
              No Anthropic API key configured.{" "}
              <Link to="/settings" className="underline">
                Add one in Settings
              </Link>{" "}
              before starting this run.
            </p>
          )}
          {isQueued && (
            <Button onClick={() => void handleStart()} disabled={isStarting || !hasApiKey}>
              {isStarting ? "Starting…" : "Start"}
            </Button>
          )}
          {isRunning && (
            <Button variant="destructive" onClick={() => void handleStop()} disabled={isStopping}>
              <Square className="mr-1.5 h-3.5 w-3.5" />
              {isStopping ? "Stopping…" : "Stop"}
            </Button>
          )}
        </div>
      </div>

      {actionError && (
        <div className="rounded-md border border-destructive/40 bg-destructive/10 px-2 py-1.5 text-xs text-destructive">
          {actionError}
        </div>
      )}

      <div className="grid flex-1 grid-cols-1 gap-4 overflow-hidden lg:grid-cols-2">
        <div className="flex flex-col gap-2 overflow-hidden">
          <h2 className="text-sm font-medium text-foreground">Activity</h2>
          <div className="flex flex-1 flex-col gap-1.5 overflow-y-auto rounded-md border border-border p-2">
            {activity.length === 0 && !isRunning && (
              <p className="text-xs text-muted-foreground">No activity yet.</p>
            )}
            {activity.length === 0 && isRunning && !streamingText && (
              <p className="text-xs text-muted-foreground">Waiting for the model's first response…</p>
            )}
            {activity.map((event) => (
              <div key={event.id} className="rounded border border-border/60 bg-surface px-2 py-1.5 text-xs">
                <div className="flex items-center justify-between gap-2">
                  <span className="font-medium text-foreground">{event.eventType.replace(/_/g, " ")}</span>
                  <span className="text-[10px] text-subtle-foreground">
                    {new Date(event.createdAt).toLocaleTimeString()}
                  </span>
                </div>
                <p className="mt-0.5 whitespace-pre-wrap text-muted-foreground">
                  {formatActivitySummary(event.payloadJson)}
                </p>
              </div>
            ))}
            {isRunning && streamingText && (
              <div className="rounded border border-primary/40 bg-primary/5 px-2 py-1.5 text-xs">
                <span className="font-medium text-foreground">thinking…</span>
                <p className="mt-0.5 whitespace-pre-wrap text-muted-foreground">{streamingText}</p>
              </div>
            )}
          </div>
        </div>

        <div className="flex flex-col gap-2 overflow-hidden">
          <h2 className="text-sm font-medium text-foreground">Tool calls</h2>
          <div className="flex flex-1 flex-col gap-1.5 overflow-y-auto rounded-md border border-border p-2">
            {toolCalls.length === 0 && (
              <p className="text-xs text-muted-foreground">
                {isTerminal ? "No tool calls — the model answered without using any tools." : "No tool calls yet."}
              </p>
            )}
            {toolCalls.map((call) => {
              const badge = TOOL_CALL_BADGE[call.status];
              return (
                <div key={call.id} className="rounded border border-border/60 bg-surface px-2 py-1.5 text-xs">
                  <div className="flex items-center justify-between gap-2">
                    <span className="font-mono font-medium text-foreground">{call.toolName}</span>
                    <div className="flex items-center gap-1.5">
                      {call.durationMs !== null && (
                        <span className="text-[10px] text-subtle-foreground">{call.durationMs}ms</span>
                      )}
                      <Badge variant={badge.variant}>{badge.label}</Badge>
                    </div>
                  </div>
                  <pre className="mt-1 overflow-x-auto whitespace-pre-wrap break-words text-[11px] text-muted-foreground">
                    {call.inputJson}
                  </pre>
                  {call.outputJson && (
                    <p className="mt-1 whitespace-pre-wrap break-words text-[11px] text-muted-foreground">
                      {call.outputJson}
                    </p>
                  )}
                </div>
              );
            })}
          </div>
        </div>
      </div>

      {isTerminal && (
        <div className="flex min-h-[240px] flex-1 flex-col gap-2 overflow-hidden">
          <h2 className="text-sm font-medium text-foreground">Changes</h2>
          <RunDiffPanel agentRunId={run.id} />
        </div>
      )}
    </div>
  );
}
