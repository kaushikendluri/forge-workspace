import { useEffect, useState } from "react";
import { ListChecks, Sparkles } from "lucide-react";
import { EmptyState } from "@/components/empty-states/EmptyState";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { useProjectStore } from "@/stores/useProjectStore";
import { approveMissionPlan, createMission, getMission, listMissionTasks, listMissions } from "@/lib/tauri";
import type { Mission, MissionStatus, Task, TaskPriority } from "@/types/db";

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
};

const TASK_PRIORITY_BADGE: Record<TaskPriority, { label: string; variant: BadgeVariant }> = {
  low: { label: "low", variant: "secondary" },
  medium: { label: "medium", variant: "default" },
  high: { label: "high", variant: "warning" },
};

/**
 * One mission's plan review card: its real tasks (`list_mission_tasks`) with
 * title/description/suggested agent type/priority/dependency, and — only
 * while `status` is `plan_ready` — a working Approve action. Approving only
 * records sign-off (`approve_mission_plan`); it never implies anything
 * starts running, since execution is a future milestone.
 */
function MissionPlanCard({ mission, onMissionUpdated }: { mission: Mission; onMissionUpdated: (mission: Mission) => void }) {
  const [tasks, setTasks] = useState<Task[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [isApproving, setIsApproving] = useState(false);
  const [approveError, setApproveError] = useState<string | null>(null);

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

  const titleById = new Map((tasks ?? []).map((t) => [t.id, t.title] as const));
  const statusBadge = MISSION_STATUS_BADGE[mission.status];

  return (
    <div className="flex flex-col gap-3 rounded-md border border-border p-3">
      <div className="flex items-start justify-between gap-3">
        <div className="flex flex-col gap-1">
          <p className="text-sm font-medium text-foreground">{mission.objective}</p>
          <p className="text-[11px] text-subtle-foreground">Created {new Date(mission.createdAt).toLocaleString()}</p>
        </div>
        <Badge variant={statusBadge.variant}>{statusBadge.label}</Badge>
      </div>

      {mission.status === "failed" && (
        <div className="rounded-md border border-destructive/40 bg-destructive/10 px-2 py-1.5 text-xs text-destructive">
          Plan generation failed: {mission.errorMessage ?? "unknown error"}
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
          {tasks.map((task) => (
            <div key={task.id} className="rounded border border-border/60 bg-surface px-2 py-1.5 text-xs">
              <div className="flex items-center justify-between gap-2">
                <span className="font-medium text-foreground">{task.title}</span>
                <div className="flex shrink-0 items-center gap-1.5">
                  {task.agentType && <Badge variant="outline">{task.agentType}</Badge>}
                  <Badge variant={TASK_PRIORITY_BADGE[task.priority].variant}>
                    {TASK_PRIORITY_BADGE[task.priority].label}
                  </Badge>
                </div>
              </div>
              {task.description && <p className="mt-1 text-muted-foreground">{task.description}</p>}
              {task.dependsOnTaskId && (
                <p className="mt-1 text-[11px] text-subtle-foreground">
                  Depends on: {titleById.get(task.dependsOnTaskId) ?? task.dependsOnTaskId}
                </p>
              )}
            </div>
          ))}
        </div>
      )}

      {mission.status === "plan_ready" && (
        <div className="flex flex-wrap items-center gap-2">
          <Button size="sm" onClick={() => void handleApprove()} disabled={isApproving}>
            {isApproving ? "Approving…" : "Approve plan"}
          </Button>
          <p className="text-[11px] text-subtle-foreground">
            Approving only records your sign-off — running these tasks is a future milestone; nothing starts yet.
          </p>
        </div>
      )}

      {mission.status === "approved" && (
        <p className="text-[11px] text-subtle-foreground">
          Approved{mission.approvedAt ? ` ${new Date(mission.approvedAt).toLocaleString()}` : ""}. This plan is saved
          and waiting — execution isn't built yet, so nothing will run on its own.
        </p>
      )}

      {approveError && (
        <div className="rounded-md border border-destructive/40 bg-destructive/10 px-2 py-1.5 text-xs text-destructive">
          {approveError}
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
 * rows. The user reviews the plan here and can approve it; nothing on this
 * page starts an agent or runs anything — execution is a future milestone.
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
        a real Anthropic call breaks it into a task plan for you to review below. Approving a plan only records your
        sign-off — running the tasks is a future milestone, not something this page does.
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
          <MissionPlanCard key={mission.id} mission={mission} onMissionUpdated={handleMissionUpdated} />
        ))}
      </div>
    </div>
  );
}
