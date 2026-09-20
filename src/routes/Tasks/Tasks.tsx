import { useEffect, useState } from "react";
import { Link } from "react-router-dom";
import { ListChecks, Play, Sparkles, Square } from "lucide-react";
import { EmptyState } from "@/components/empty-states/EmptyState";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { useProjectStore } from "@/stores/useProjectStore";
import {
  approveMissionPlan,
  createMission,
  getAgentRun,
  getMission,
  listMissionTasks,
  listMissions,
  startMission,
  stopMission,
} from "@/lib/tauri";
import { onForgeEvent } from "@/lib/events";
import type { Mission, MissionStatus, Task, TaskPriority, TaskStatus } from "@/types/db";

function errorMessage(err: unknown): string {
  if (typeof err === "string") return err;
  if (err instanceof Error) return err.message;
  return String(err);
}

type BadgeVariant = "default" | "success" | "destructive" | "warning" | "secondary";

const MISSION_STATUS_BADGE: Record<MissionStatus, { label: string; variant: BadgeVariant }> = {
  planning: { label: "Planning…", variant: "secondary" },
  plan_ready: { label: "Plan ready", variant: "default" },
  approved: { label: "Approved", variant: "success" },
  running: { label: "Running", variant: "default" },
  completed: { label: "Completed", variant: "success" },
  failed: { label: "Failed", variant: "destructive" },
  stopped: { label: "Stopped", variant: "warning" },
};

const TASK_PRIORITY_BADGE: Record<TaskPriority, { label: string; variant: BadgeVariant }> = {
  low: { label: "low", variant: "secondary" },
  medium: { label: "medium", variant: "default" },
  high: { label: "high", variant: "warning" },
};

/**
 * M9: a task's live execution status, as the scheduler
 * (`orchestrator::scheduler`) drives it. `backlog`/`todo` both read as
 * "not started yet" here — the plan never produces `todo` tasks, only
 * `backlog` ones, but the badge map stays total over `TaskStatus`.
 */
const TASK_STATUS_BADGE: Record<TaskStatus, { label: string; variant: BadgeVariant }> = {
  backlog: { label: "Not started", variant: "secondary" },
  todo: { label: "Not started", variant: "secondary" },
  in_progress: { label: "Running", variant: "default" },
  done: { label: "Done", variant: "success" },
  failed: { label: "Failed", variant: "destructive" },
  blocked: { label: "Blocked", variant: "warning" },
  cancelled: { label: "Cancelled", variant: "secondary" },
};

/**
 * Resolves `agentRunId` to the agent that owns it and links to the real
 * `AgentDetail` page for it — the same activity feed / tool calls / diff
 * view the Agents flow uses, never a parallel view. `Task` only stores
 * `agentRunId` (not `agentId`), so this fetches the run once to find its
 * owning agent.
 */
function TaskRunLink({ agentRunId, projectId }: { agentRunId: string; projectId: string }) {
  const [agentId, setAgentId] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setAgentId(null);
    getAgentRun(agentRunId)
      .then((run) => {
        if (!cancelled) setAgentId(run.agentId);
      })
      .catch(() => {
        // Best-effort — if the run can't be resolved yet, simply show no
        // link rather than an error inline in a task row.
      });
    return () => {
      cancelled = true;
    };
  }, [agentRunId]);

  if (!agentId) return null;

  return (
    <Link to={`/projects/${projectId}/agents/${agentId}`} className="text-[11px] font-medium text-primary underline">
      View run
    </Link>
  );
}

/**
 * One mission's plan review + live execution card: its real tasks
 * (`list_mission_tasks`) with title/description/suggested agent
 * type/priority/dependency/live status, an Approve action while
 * `plan_ready`, and — once `approved` — a real "Start mission" action that
 * spawns `orchestrator::scheduler::run_mission` (M9; M10 made it run ready
 * tasks concurrently). While `running`, task statuses update live from
 * `mission:task-updated` events — every task in `tasks` renders its own
 * independent status badge, so 2+ tasks showing `Running` at once (M10)
 * "just works" the same way a single one did under M9, with no singular
 * "the current task" state anywhere in this component — and a Stop action
 * cancels the mission (all of its currently-running tasks, not just one); a
 * terminal task's agent run links to the real `AgentDetail` page.
 */
function MissionPlanCard({
  mission,
  projectId,
  onMissionUpdated,
}: {
  mission: Mission;
  projectId: string;
  onMissionUpdated: (mission: Mission) => void;
}) {
  const [tasks, setTasks] = useState<Task[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [isApproving, setIsApproving] = useState(false);
  const [approveError, setApproveError] = useState<string | null>(null);
  const [isStarting, setIsStarting] = useState(false);
  const [isStopping, setIsStopping] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  // Ephemeral: the scheduler surfaces *why* a task got blocked (cycle, a
  // failed dependency) live via the event payload, but never persists it on
  // the `tasks` row — so this is lost on reload, by design.
  const [blockedReasons, setBlockedReasons] = useState<Record<string, string>>({});

  const refetchTasks = () => {
    listMissionTasks(mission.id)
      .then((result) => setTasks(result))
      .catch((err) => setLoadError(errorMessage(err)));
  };

  useEffect(() => {
    let cancelled = false;
    setTasks(null);
    setLoadError(null);
    listMissionTasks(mission.id)
      .then((result) => {
        if (!cancelled) setTasks(result);
      })
      .catch((err) => {
        if (!cancelled) setLoadError(errorMessage(err));
      });
    return () => {
      cancelled = true;
    };
  }, [mission.id]);

  // Live updates while this mission is executing (or has just finished).
  useEffect(() => {
    let cancelled = false;
    const unlisten: Array<() => void> = [];

    void (async () => {
      unlisten.push(
        await onForgeEvent("mission:task-updated", (payload) => {
          if (cancelled || payload.missionId !== mission.id) return;
          if (payload.reason) {
            setBlockedReasons((prev) => ({ ...prev, [payload.taskId]: payload.reason as string }));
          }
          // Refetch rather than patch a partial row client-side, so
          // `updatedAt`/`agentRunId` stay authoritative too.
          refetchTasks();
        }),
      );
      unlisten.push(
        await onForgeEvent("mission:status-changed", (payload) => {
          if (cancelled || payload.missionId !== mission.id) return;
          getMission(mission.id)
            .then((refreshed) => {
              if (!cancelled) onMissionUpdated(refreshed);
            })
            .catch(() => {
              // Best-effort — the next natural refresh will catch up.
            });
        }),
      );
    })();

    return () => {
      cancelled = true;
      unlisten.forEach((fn) => fn());
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [mission.id]);

  const handleApprove = async () => {
    setIsApproving(true);
    setApproveError(null);
    try {
      await approveMissionPlan(mission.id);
      // Re-fetch rather than fabricate the new state client-side, so
      // `approvedAt` (and anything else) reflects what the backend actually
      // stored.
      const refreshed = await getMission(mission.id);
      onMissionUpdated(refreshed);
    } catch (err) {
      setApproveError(errorMessage(err));
    } finally {
      setIsApproving(false);
    }
  };

  const handleStart = async () => {
    setActionError(null);
    setIsStarting(true);
    try {
      await startMission(mission.id);
      // `start_mission` only registers the run and spawns it — the actual
      // `status = 'running'` transition (and its event) happens inside the
      // scheduler itself, so the "Starting…" state naturally clears once
      // the `mission:status-changed` event above updates `mission.status`.
    } catch (err) {
      setActionError(errorMessage(err));
      setIsStarting(false);
    }
  };

  const handleStop = async () => {
    setActionError(null);
    setIsStopping(true);
    try {
      await stopMission(mission.id);
    } catch (err) {
      setActionError(errorMessage(err));
    } finally {
      setIsStopping(false);
    }
  };

  const titleById = new Map((tasks ?? []).map((t) => [t.id, t.title] as const));
  const statusBadge = MISSION_STATUS_BADGE[mission.status];
  const isRunning = mission.status === "running";
  // A `failed` mission is either a planning failure (never got approved —
  // `orchestrator::planner` errored) or an execution failure (the scheduler
  // ran and something didn't complete) — `approvedAt` tells them apart, so
  // the message shown doesn't misattribute one for the other.
  const isExecutionFailure = mission.status === "failed" && mission.approvedAt !== null;

  return (
    <div className="flex flex-col gap-3 rounded-md border border-border p-3">
      <div className="flex items-start justify-between gap-3">
        <div className="flex flex-col gap-1">
          <p className="text-sm font-medium text-foreground">{mission.objective}</p>
          <p className="text-[11px] text-subtle-foreground">Created {new Date(mission.createdAt).toLocaleString()}</p>
        </div>
        <Badge variant={statusBadge.variant}>{statusBadge.label}</Badge>
      </div>

      {mission.status === "failed" && !isExecutionFailure && (
        <div className="rounded-md border border-destructive/40 bg-destructive/10 px-2 py-1.5 text-xs text-destructive">
          Plan generation failed: {mission.errorMessage ?? "unknown error"}
        </div>
      )}

      {isExecutionFailure && (
        <div className="rounded-md border border-destructive/40 bg-destructive/10 px-2 py-1.5 text-xs text-destructive">
          Mission didn't complete: {mission.errorMessage ?? "unknown error"}
        </div>
      )}

      {mission.status === "stopped" && (
        <div className="rounded-md border border-warning/40 bg-warning/10 px-2 py-1.5 text-xs text-warning">
          Mission stopped before every task finished.
        </div>
      )}

      {loadError && (
        <div className="rounded-md border border-destructive/40 bg-destructive/10 px-2 py-1.5 text-xs text-destructive">
          Couldn't load this mission's tasks: {loadError}
        </div>
      )}

      {tasks && tasks.length === 0 && mission.status !== "failed" && (
        <p className="text-xs text-muted-foreground">
          The model proposed no tasks for this objective. Try rephrasing it, or start a new mission.
        </p>
      )}

      {tasks && tasks.length > 0 && (
        <div className="flex flex-col gap-1.5">
          {tasks.map((task) => {
            const taskStatusBadge = TASK_STATUS_BADGE[task.status];
            const hasRun = task.agentRunId !== null && task.status !== "backlog" && task.status !== "todo";
            const reason = blockedReasons[task.id];
            return (
              <div key={task.id} className="rounded border border-border/60 bg-surface px-2 py-1.5 text-xs">
                <div className="flex items-center justify-between gap-2">
                  <span className="font-medium text-foreground">{task.title}</span>
                  <div className="flex shrink-0 items-center gap-1.5">
                    {task.agentType && <Badge variant="outline">{task.agentType}</Badge>}
                    <Badge variant={TASK_PRIORITY_BADGE[task.priority].variant}>
                      {TASK_PRIORITY_BADGE[task.priority].label}
                    </Badge>
                    <Badge variant={taskStatusBadge.variant}>{taskStatusBadge.label}</Badge>
                  </div>
                </div>
                {task.description && <p className="mt-1 text-muted-foreground">{task.description}</p>}
                {task.dependsOnTaskId && (
                  <p className="mt-1 text-[11px] text-subtle-foreground">
                    Depends on: {titleById.get(task.dependsOnTaskId) ?? task.dependsOnTaskId}
                  </p>
                )}
                {task.status === "blocked" && reason && (
                  <p className="mt-1 text-[11px] text-warning">Blocked: {reason}</p>
                )}
                {hasRun && task.agentRunId && (
                  <div className="mt-1.5">
                    <TaskRunLink agentRunId={task.agentRunId} projectId={projectId} />
                  </div>
                )}
              </div>
            );
          })}
        </div>
      )}

      {mission.status === "plan_ready" && (
        <div className="flex flex-wrap items-center gap-2">
          <Button size="sm" onClick={() => void handleApprove()} disabled={isApproving}>
            {isApproving ? "Approving…" : "Approve plan"}
          </Button>
          <p className="text-[11px] text-subtle-foreground">
            Approving records your sign-off. You'll be able to start real execution once it's approved.
          </p>
        </div>
      )}

      {mission.status === "approved" && (
        <div className="flex flex-wrap items-center gap-2">
          <Button size="sm" onClick={() => void handleStart()} disabled={isStarting}>
            <Play className="mr-1.5 h-3.5 w-3.5" />
            {isStarting ? "Starting…" : "Start mission"}
          </Button>
          <p className="text-[11px] text-subtle-foreground">
            Approved{mission.approvedAt ? ` ${new Date(mission.approvedAt).toLocaleString()}` : ""}. Starting runs
            ready tasks through real agents concurrently, in dependency order.
          </p>
        </div>
      )}

      {isRunning && (
        <div className="flex flex-wrap items-center gap-2">
          <Button size="sm" variant="destructive" onClick={() => void handleStop()} disabled={isStopping}>
            <Square className="mr-1.5 h-3.5 w-3.5" />
            {isStopping ? "Stopping…" : "Stop mission"}
          </Button>
          <p className="text-[11px] text-subtle-foreground">
            Ready tasks run concurrently, up to the configured limit, in dependency order.
          </p>
        </div>
      )}

      {approveError && (
        <div className="rounded-md border border-destructive/40 bg-destructive/10 px-2 py-1.5 text-xs text-destructive">
          {approveError}
        </div>
      )}

      {actionError && (
        <div className="rounded-md border border-destructive/40 bg-destructive/10 px-2 py-1.5 text-xs text-destructive">
          {actionError}
        </div>
      )}
    </div>
  );
}

/**
 * Real Mission Control for the active project: a plain-English objective is
 * sent to `create_mission`, which makes one real Anthropic call
 * (`orchestrator::planner`, forced structured tool output — not the M6
 * multi-turn agent loop) and returns a structured plan as real `tasks`
 * rows. The user reviews the plan, approves it, and can start real
 * execution: `orchestrator::scheduler::run_mission` walks the task
 * dependency graph and runs ready tasks concurrently (M10; bounded by the
 * `agent.max_parallel_agents` setting) through the real M5/M6 agent
 * pipeline, promoting dependent tasks as soon as what they depend on
 * finishes, with live status and a Stop action.
 */
export function Tasks() {
  const activeProjectId = useProjectStore((s) => s.activeProjectId);
  const activeProject = useProjectStore((s) => s.projects.find((p) => p.id === activeProjectId));

  const [missions, setMissions] = useState<Mission[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [listError, setListError] = useState<string | null>(null);

  const [objective, setObjective] = useState("");
  const [isGenerating, setIsGenerating] = useState(false);
  const [generateError, setGenerateError] = useState<string | null>(null);

  useEffect(() => {
    if (!activeProjectId) {
      setMissions([]);
      return;
    }
    let cancelled = false;
    setIsLoading(true);
    setListError(null);
    listMissions(activeProjectId)
      .then((result) => {
        if (!cancelled) setMissions(result);
      })
      .catch((err) => {
        if (!cancelled) setListError(errorMessage(err));
      })
      .finally(() => {
        if (!cancelled) setIsLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [activeProjectId]);

  const handleGenerate = async () => {
    if (!activeProjectId) return;
    const trimmed = objective.trim();
    if (!trimmed) return;
    setIsGenerating(true);
    setGenerateError(null);
    try {
      // A real Anthropic call — this can genuinely take several seconds,
      // there's no faked instant completion here.
      const mission = await createMission(activeProjectId, trimmed);
      setMissions((prev) => [mission, ...prev]);
      setObjective("");
    } catch (err) {
      setGenerateError(errorMessage(err));
    } finally {
      setIsGenerating(false);
    }
  };

  const handleMissionUpdated = (updated: Mission) => {
    setMissions((prev) => prev.map((m) => (m.id === updated.id ? updated : m)));
  };

  if (!activeProject) {
    return (
      <div className="flex flex-1 flex-col gap-4">
        <h1 className="text-lg font-semibold text-foreground">Tasks</h1>
        <EmptyState icon={ListChecks} title="No tasks yet" description="Open a project to get started." />
      </div>
    );
  }

  return (
    <div className="flex flex-1 flex-col gap-4 overflow-hidden">
      <h1 className="text-lg font-semibold text-foreground">Tasks</h1>
      <p className="max-w-2xl text-xs text-muted-foreground">
        Give <span className="font-medium text-foreground">{activeProject.name}</span> a plain-English objective and
        a real Anthropic call breaks it into a task plan for you to review below. Once approved, starting the
        mission runs ready tasks through real agents concurrently (each in its own isolated git worktree, bounded by
        a configurable limit), promoting dependent tasks as soon as what they depend on finishes.
      </p>

      <div className="flex flex-col gap-2">
        <textarea
          value={objective}
          onChange={(e) => setObjective(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) void handleGenerate();
          }}
          placeholder="e.g. Add dark mode support across the app, including a settings toggle that persists the choice"
          rows={3}
          disabled={isGenerating}
          className="w-full resize-y rounded-md border border-border bg-background-elevated px-3 py-2 text-sm text-foreground shadow-none transition-colors placeholder:text-subtle-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-primary disabled:cursor-not-allowed disabled:opacity-50"
        />
        <div>
          <Button onClick={() => void handleGenerate()} disabled={isGenerating || !objective.trim()}>
            <Sparkles className="mr-1.5 h-3.5 w-3.5" />
            {isGenerating ? "Generating plan…" : "Generate plan"}
          </Button>
        </div>
      </div>

      {generateError && (
        <div className="rounded-md border border-destructive/40 bg-destructive/10 px-2 py-1.5 text-xs text-destructive">
          {generateError}
        </div>
      )}

      {listError && (
        <div className="rounded-md border border-destructive/40 bg-destructive/10 px-2 py-1.5 text-xs text-destructive">
          {listError}
        </div>
      )}

      {!isLoading && !listError && missions.length === 0 && (
        <EmptyState
          icon={ListChecks}
          title="No missions yet"
          description="Describe an objective above and generate a plan to get started."
        />
      )}

      <div className="flex flex-1 flex-col gap-3 overflow-y-auto">
        {missions.map((mission) => (
          <MissionPlanCard
            key={mission.id}
            mission={mission}
            projectId={activeProject.id}
            onMissionUpdated={handleMissionUpdated}
          />
        ))}
      </div>
    </div>
  );
}
