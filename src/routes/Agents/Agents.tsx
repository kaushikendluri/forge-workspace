import { useEffect, useState } from "react";
import { Bot } from "lucide-react";
import { EmptyState } from "@/components/empty-states/EmptyState";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { useProjectStore } from "@/stores/useProjectStore";
import { createAgent, listAgents, startWorktreeForAgent } from "@/lib/tauri";
import type { Agent, Workspace } from "@/types/db";

function errorMessage(err: unknown): string {
  if (typeof err === "string") return err;
  if (err instanceof Error) return err.message;
  return String(err);
}

/**
 * Real Agents page for the active project: agents come from `list_agents`
 * (SQLite-backed), "New agent" creates a row via `create_agent`, and each
 * agent's "Start" button calls `start_worktree_for_agent` — this milestone's
 * only real capability is creating an isolated git worktree/branch for a
 * task. There is no model call or tool loop yet (that's a later milestone),
 * so the result is reported as a workspace being ready, never as the agent
 * "running" or "thinking".
 */
export function Agents() {
  const activeProjectId = useProjectStore((s) => s.activeProjectId);
  const activeProject = useProjectStore((s) => s.projects.find((p) => p.id === activeProjectId));

  const [agents, setAgents] = useState<Agent[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [listError, setListError] = useState<string | null>(null);

  const [newAgentName, setNewAgentName] = useState("");
  const [isCreating, setIsCreating] = useState(false);
  const [createError, setCreateError] = useState<string | null>(null);

  const [startingAgentId, setStartingAgentId] = useState<string | null>(null);
  const [startErrors, setStartErrors] = useState<Record<string, string>>({});
  const [workspacesByAgentId, setWorkspacesByAgentId] = useState<Record<string, Workspace>>({});

  useEffect(() => {
    if (!activeProjectId) {
      setAgents([]);
      return;
    }
    let cancelled = false;
    setIsLoading(true);
    setListError(null);
    listAgents(activeProjectId)
      .then((result) => {
        if (!cancelled) setAgents(result);
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

  const handleCreateAgent = async () => {
    if (!activeProjectId) return;
    const name = newAgentName.trim();
    if (!name) return;
    setIsCreating(true);
    setCreateError(null);
    try {
      const agent = await createAgent(activeProjectId, name);
      setAgents((prev) => [agent, ...prev]);
      setNewAgentName("");
    } catch (err) {
      setCreateError(errorMessage(err));
    } finally {
      setIsCreating(false);
    }
  };

  const handleStart = async (agent: Agent) => {
    const taskPrompt = window.prompt(`What should "${agent.name}" work on?`)?.trim();
    if (!taskPrompt) return;
    setStartingAgentId(agent.id);
    setStartErrors((prev) => ({ ...prev, [agent.id]: "" }));
    try {
      const workspace = await startWorktreeForAgent(agent.id, taskPrompt);
      setWorkspacesByAgentId((prev) => ({ ...prev, [agent.id]: workspace }));
    } catch (err) {
      setStartErrors((prev) => ({ ...prev, [agent.id]: errorMessage(err) }));
    } finally {
      setStartingAgentId(null);
    }
  };

  if (!activeProject) {
    return (
      <div className="flex flex-1 flex-col gap-4">
        <h1 className="text-lg font-semibold text-foreground">Agents</h1>
        <EmptyState icon={Bot} title="No agents yet" description="Open a project to get started." />
      </div>
    );
  }

  return (
    <div className="flex flex-1 flex-col gap-4">
      <h1 className="text-lg font-semibold text-foreground">Agents</h1>
      <p className="text-xs text-muted-foreground">
        Agents for <span className="font-medium text-foreground">{activeProject.name}</span>. "Start" only creates
        an isolated git worktree and branch for a task — agent execution lands in a later milestone.
      </p>

      <div className="flex items-center gap-2">
        <Input
          value={newAgentName}
          onChange={(e) => setNewAgentName(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") void handleCreateAgent();
          }}
          placeholder="Agent name"
          className="w-64"
        />
        <Button onClick={() => void handleCreateAgent()} disabled={isCreating || !newAgentName.trim()}>
          New agent
        </Button>
      </div>
      {createError && (
        <div className="rounded-md border border-destructive/40 bg-destructive/10 px-2 py-1.5 text-xs text-destructive">
          {createError}
        </div>
      )}

      {listError && (
        <div className="rounded-md border border-destructive/40 bg-destructive/10 px-2 py-1.5 text-xs text-destructive">
          {listError}
        </div>
      )}

      {!isLoading && !listError && agents.length === 0 && (
        <EmptyState icon={Bot} title="No agents yet" description="Create one above to get started." />
      )}

      <div className="flex flex-col gap-2">
        {agents.map((agent) => {
          const workspace = workspacesByAgentId[agent.id];
          const startError = startErrors[agent.id];
          return (
            <div key={agent.id} className="flex flex-col gap-2 rounded-md border border-border p-3">
              <div className="flex items-center justify-between gap-2">
                <div className="flex flex-col">
                  <span className="text-sm font-medium text-foreground">{agent.name}</span>
                  <span className="text-xs text-muted-foreground">status: {agent.status}</span>
                </div>
                <Button
                  variant="secondary"
                  size="sm"
                  onClick={() => void handleStart(agent)}
                  disabled={startingAgentId === agent.id}
                >
                  {startingAgentId === agent.id ? "Creating workspace…" : "Start"}
                </Button>
              </div>
              {startError && (
                <div className="rounded-md border border-destructive/40 bg-destructive/10 px-2 py-1.5 text-xs text-destructive">
                  {startError}
                </div>
              )}
              {workspace && (
                <p className="text-xs text-muted-foreground">
                  Workspace ready at <code className="rounded bg-surface px-1 py-0.5 font-mono">{workspace.path}</code>{" "}
                  on branch <code className="rounded bg-surface px-1 py-0.5 font-mono">{workspace.branchName}</code>.
                  Agent execution lands in a later milestone.
                </p>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}
