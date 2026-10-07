import { useEffect, useMemo, useState } from "react";
import { Sparkles } from "lucide-react";
import { EmptyState } from "@/components/empty-states/EmptyState";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Separator } from "@/components/ui/separator";
import {
  createAgentSkill,
  deleteAgentSkill,
  duplicateAgentSkill,
  listAgentSkills,
  listAvailableTools,
  listModelConfigs,
  updateAgentSkill,
} from "@/lib/tauri";
import { ALL_TOOLS_SENTINEL, type AgentSkill, type ModelConfig, type ToolCatalogueEntryDto } from "@/types/db";

function errorMessage(err: unknown): string {
  if (typeof err === "string") return err;
  if (err instanceof Error) return err.message;
  return String(err);
}

/** Parses an `AgentSkill.toolsJson` string into `{ allTools, names }`. */
function parseToolsJson(toolsJson: string): { allTools: boolean; names: Set<string> } {
  try {
    const parsed: unknown = JSON.parse(toolsJson);
    if (!Array.isArray(parsed)) return { allTools: true, names: new Set() };
    if (parsed.includes(ALL_TOOLS_SENTINEL)) return { allTools: true, names: new Set() };
    return { allTools: false, names: new Set(parsed.filter((v): v is string => typeof v === "string")) };
  } catch {
    return { allTools: true, names: new Set() };
  }
}

/** Builds the `toolsJson` string this form state should be saved as. */
function buildToolsJson(allTools: boolean, names: Set<string>): string {
  return allTools ? JSON.stringify([ALL_TOOLS_SENTINEL]) : JSON.stringify(Array.from(names));
}

interface SkillFormState {
  name: string;
  description: string;
  instructions: string;
  allTools: boolean;
  toolNames: Set<string>;
  preferredModelId: string;
}

const EMPTY_FORM: SkillFormState = {
  name: "",
  description: "",
  instructions: "",
  allTools: true,
  toolNames: new Set(),
  preferredModelId: "",
};

function skillToForm(skill: AgentSkill): SkillFormState {
  const { allTools, names } = parseToolsJson(skill.toolsJson);
  return {
    name: skill.name,
    description: skill.description ?? "",
    instructions: skill.instructions,
    allTools,
    toolNames: names,
    preferredModelId: skill.preferredModelId ?? "",
  };
}

/**
 * Phase 5 M19: real Agent Skills management — list, create, edit, duplicate,
 * delete. The tool checklist is built from `listAvailableTools` (the real
 * backend catalogue, `agent::schema::all_tool_definitions`), never a
 * hand-duplicated frontend list, so it can never drift from what the
 * backend actually validates/offers. The model dropdown reuses
 * `listModelConfigs` (M4) — no new model/provider mechanism here.
 */
export function Skills() {
  const [skills, setSkills] = useState<AgentSkill[]>([]);
  const [tools, setTools] = useState<ToolCatalogueEntryDto[]>([]);
  const [models, setModels] = useState<ModelConfig[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [listError, setListError] = useState<string | null>(null);

  const [editingSkillId, setEditingSkillId] = useState<string | null>(null);
  const [form, setForm] = useState<SkillFormState>(EMPTY_FORM);
  const [isSaving, setIsSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);

  const isEditing = editingSkillId !== null;

  const loadSkills = () => {
    setIsLoading(true);
    setListError(null);
    return listAgentSkills()
      .then(setSkills)
      .catch((err) => setListError(errorMessage(err)))
      .finally(() => setIsLoading(false));
  };

  useEffect(() => {
    void loadSkills();
    listAvailableTools()
      .then(setTools)
      .catch(() => setTools([]));
    listModelConfigs()
      .then(setModels)
      .catch(() => setModels([]));
  }, []);

  const modelById = useMemo(() => new Map(models.map((m) => [m.id, m])), [models]);

  const startCreate = () => {
    setEditingSkillId(null);
    setForm(EMPTY_FORM);
    setSaveError(null);
  };

  const startEdit = (skill: AgentSkill) => {
    setEditingSkillId(skill.id);
    setForm(skillToForm(skill));
    setSaveError(null);
  };

  const toggleTool = (name: string) => {
    setForm((prev) => {
      const next = new Set(prev.toolNames);
      if (next.has(name)) next.delete(name);
      else next.add(name);
      return { ...prev, toolNames: next };
    });
  };

  const handleSave = async () => {
    const name = form.name.trim();
    const instructions = form.instructions.trim();
    if (!name || !instructions) return;

    setIsSaving(true);
    setSaveError(null);
    try {
      const description = form.description.trim() || null;
      const toolsJson = buildToolsJson(form.allTools, form.toolNames);
      const preferredModelId = form.preferredModelId || null;

      if (editingSkillId) {
        const updated = await updateAgentSkill(editingSkillId, name, description, instructions, toolsJson, preferredModelId);
        setSkills((prev) => prev.map((s) => (s.id === updated.id ? updated : s)));
      } else {
        const created = await createAgentSkill(name, description, instructions, toolsJson, preferredModelId);
        setSkills((prev) => [...prev, created]);
      }
      startCreate();
    } catch (err) {
      setSaveError(errorMessage(err));
    } finally {
      setIsSaving(false);
    }
  };

  const handleDelete = async (skill: AgentSkill) => {
    setActionError(null);
    try {
      await deleteAgentSkill(skill.id);
      setSkills((prev) => prev.filter((s) => s.id !== skill.id));
      if (editingSkillId === skill.id) startCreate();
    } catch (err) {
      setActionError(errorMessage(err));
    }
  };

  const handleDuplicate = async (skill: AgentSkill) => {
    setActionError(null);
    try {
      const copy = await duplicateAgentSkill(skill.id);
      setSkills((prev) => [...prev, copy]);
    } catch (err) {
      setActionError(errorMessage(err));
    }
  };

  return (
    <div className="flex flex-1 flex-col gap-4">
      <h1 className="text-lg font-semibold text-foreground">Skills</h1>
      <p className="text-xs text-muted-foreground">
        Reusable, named specialist agent configurations — instructions, allowed tools, and a preferred model. Select
        one when creating a new agent (Agents page) to apply it for real.
      </p>

      <div className="flex flex-col gap-3 rounded-md border border-border p-3">
        <h2 className="text-sm font-semibold text-foreground">{isEditing ? "Edit skill" : "New skill"}</h2>

        <div className="flex flex-col gap-1">
          <label className="text-xs font-medium text-muted-foreground">Name</label>
          <Input
            value={form.name}
            onChange={(e) => setForm((prev) => ({ ...prev, name: e.target.value }))}
            placeholder="e.g. Senior React Engineer"
            className="max-w-sm"
          />
        </div>

        <div className="flex flex-col gap-1">
          <label className="text-xs font-medium text-muted-foreground">Description</label>
          <Input
            value={form.description}
            onChange={(e) => setForm((prev) => ({ ...prev, description: e.target.value }))}
            placeholder="Short summary shown in the skill list"
            className="max-w-md"
          />
        </div>

        <div className="flex flex-col gap-1">
          <label className="text-xs font-medium text-muted-foreground">Instructions</label>
          <textarea
            value={form.instructions}
            onChange={(e) => setForm((prev) => ({ ...prev, instructions: e.target.value }))}
            placeholder="Added to every agent created from this skill's system prompt."
            rows={4}
            className="flex w-full max-w-xl rounded-md border border-border bg-background-elevated px-3 py-2 text-sm text-foreground shadow-none transition-colors placeholder:text-subtle-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-primary"
          />
        </div>

        <div className="flex flex-col gap-1">
          <label className="text-xs font-medium text-muted-foreground">Preferred model</label>
          <select
            value={form.preferredModelId}
            onChange={(e) => setForm((prev) => ({ ...prev, preferredModelId: e.target.value }))}
            className="h-9 max-w-xs rounded-md border border-border bg-background px-2 text-sm text-foreground"
          >
            <option value="">No preference (use project default)</option>
            {models.map((model) => (
              <option key={model.id} value={model.id}>
                {model.displayName}
              </option>
            ))}
          </select>
        </div>

        <div className="flex flex-col gap-1.5">
          <label className="text-xs font-medium text-muted-foreground">Allowed tools</label>
          <label className="flex items-center gap-2 text-sm text-foreground">
            <input
              type="checkbox"
              checked={form.allTools}
              onChange={(e) => setForm((prev) => ({ ...prev, allTools: e.target.checked }))}
            />
            All tools (unrestricted — today's default agent behavior)
          </label>
          {!form.allTools && (
            <div className="grid max-w-xl grid-cols-2 gap-1 rounded-md border border-border bg-surface p-2">
              {tools.map((tool) => (
                <label key={tool.name} className="flex items-center gap-2 text-xs text-foreground" title={tool.description}>
                  <input type="checkbox" checked={form.toolNames.has(tool.name)} onChange={() => toggleTool(tool.name)} />
                  <code className="font-mono">{tool.name}</code>
                </label>
              ))}
              {tools.length === 0 && <span className="text-xs text-muted-foreground">Loading tool catalogue…</span>}
            </div>
          )}
        </div>

        {saveError && (
          <div className="rounded-md border border-destructive/40 bg-destructive/10 px-2 py-1.5 text-xs text-destructive">
            {saveError}
          </div>
        )}

        <div className="flex items-center gap-2">
          <Button
            onClick={() => void handleSave()}
            disabled={isSaving || !form.name.trim() || !form.instructions.trim()}
          >
            {isSaving ? "Saving…" : isEditing ? "Save changes" : "Create skill"}
          </Button>
          {isEditing && (
            <Button variant="outline" onClick={startCreate} disabled={isSaving}>
              Cancel
            </Button>
          )}
        </div>
      </div>

      <Separator />

      {listError && (
        <div className="rounded-md border border-destructive/40 bg-destructive/10 px-2 py-1.5 text-xs text-destructive">
          {listError}
        </div>
      )}
      {actionError && (
        <div className="rounded-md border border-destructive/40 bg-destructive/10 px-2 py-1.5 text-xs text-destructive">
          {actionError}
        </div>
      )}

      {!isLoading && !listError && skills.length === 0 && (
        <EmptyState icon={Sparkles} title="No skills yet" description="Create one above to get started." />
      )}

      <div className="flex flex-col gap-2">
        {skills.map((skill) => {
          const { allTools, names } = parseToolsJson(skill.toolsJson);
          const preferredModel = skill.preferredModelId ? modelById.get(skill.preferredModelId) : undefined;
          return (
            <div key={skill.id} className="flex flex-col gap-2 rounded-md border border-border p-3">
              <div className="flex items-center justify-between gap-2">
                <div className="flex flex-col">
                  <span className="text-sm font-medium text-foreground">{skill.name}</span>
                  {skill.description && <span className="text-xs text-muted-foreground">{skill.description}</span>}
                </div>
                <div className="flex items-center gap-2">
                  <Button variant="outline" size="sm" onClick={() => startEdit(skill)}>
                    Edit
                  </Button>
                  <Button variant="outline" size="sm" onClick={() => void handleDuplicate(skill)}>
                    Duplicate
                  </Button>
                  <Button variant="destructive" size="sm" onClick={() => void handleDelete(skill)}>
                    Delete
                  </Button>
                </div>
              </div>
              <div className="flex flex-wrap items-center gap-1.5">
                <Badge variant={allTools ? "outline" : "secondary"}>
                  {allTools ? "All tools" : `${names.size} tool${names.size === 1 ? "" : "s"}`}
                </Badge>
                {preferredModel && <Badge variant="outline">{preferredModel.displayName}</Badge>}
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}
