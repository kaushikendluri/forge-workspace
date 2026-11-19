import { useEffect, useState, type ReactNode } from "react";
import { Moon, Sun } from "lucide-react";
import { cn } from "@/lib/utils";
import { useSettingsStore, type Theme } from "@/stores/useSettingsStore";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Separator } from "@/components/ui/separator";
import {
  clearProviderApiKey,
  getSetting,
  hasProviderApiKey,
  isTauriRuntime,
  listModelConfigs,
  setProviderApiKey,
  setSetting,
} from "@/lib/tauri";
import type { ModelConfig } from "@/types/db";

function SettingsSection({
  title,
  description,
  children,
}: {
  title: string;
  description?: string;
  children: ReactNode;
}) {
  return (
    <section className="flex flex-col gap-3">
      <div>
        <h2 className="text-sm font-semibold text-foreground">{title}</h2>
        {description && <p className="mt-0.5 text-xs text-muted-foreground">{description}</p>}
      </div>
      {children}
    </section>
  );
}

const themeOptions: { value: Theme; label: string; icon: typeof Sun }[] = [
  { value: "dark", label: "Dark", icon: Moon },
  { value: "light", label: "Light", icon: Sun },
];

/**
 * API key input + Save/Clear, wired to the real `set_api_key`/`has_api_key`/
 * `clear_api_key` commands (OS keychain-backed). The stored key's plaintext
 * value is never re-read into the UI — only whether one is configured.
 */
function ApiKeySection() {
  const hasKey = useSettingsStore((s) => s.hasApiKey);
  const isChecking = useSettingsStore((s) => s.isCheckingApiKey);
  const apiKeyError = useSettingsStore((s) => s.apiKeyError);
  const checkApiKey = useSettingsStore((s) => s.checkApiKey);
  const setApiKey = useSettingsStore((s) => s.setApiKey);
  const clearApiKey = useSettingsStore((s) => s.clearApiKey);

  const [input, setInput] = useState("");
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    void checkApiKey();
  }, [checkApiKey]);

  const handleSave = async () => {
    setSaving(true);
    const ok = await setApiKey(input);
    setSaving(false);
    if (ok) setInput("");
  };

  const handleClear = async () => {
    setSaving(true);
    await clearApiKey();
    setSaving(false);
  };

  if (!isTauriRuntime()) {
    return (
      <p className="text-xs text-muted-foreground">
        API key storage requires the desktop app runtime.
      </p>
    );
  }

  if (isChecking) {
    return <p className="text-xs text-muted-foreground">Checking keychain…</p>;
  }

  if (hasKey) {
    return (
      <div className="flex items-center justify-between gap-3 rounded-md border border-border bg-surface px-3 py-2.5">
        <div className="flex items-center gap-2">
          <Badge variant="success">Key configured</Badge>
          <span className="text-xs text-muted-foreground">
            Anthropic API key stored in the OS keychain.
          </span>
        </div>
        <Button variant="outline" size="sm" onClick={() => void handleClear()} disabled={saving}>
          Clear
        </Button>
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-2">
      <div className="flex gap-2">
        <Input
          type="password"
          placeholder="sk-ant-..."
          value={input}
          onChange={(e) => setInput(e.target.value)}
          autoComplete="off"
          className="max-w-xs"
        />
        <Button onClick={() => void handleSave()} disabled={saving || input.trim().length === 0}>
          Save
        </Button>
      </div>
      <p className="text-xs text-muted-foreground">
        No key set. Stored in the OS keychain — never in the database.
      </p>
      {apiKeyError && <p className="text-xs text-destructive">{apiKeyError}</p>}
    </div>
  );
}

/**
 * Phase 5 M20: the same key + Save/Clear pattern `ApiKeySection` already
 * established for Anthropic, generalized by `provider`/`label` for the
 * other three real providers (`agent::provider` now has a real
 * `ModelProvider` impl for each). Kept as its own local-state component
 * (rather than folded into the Anthropic-specific, store-backed
 * `ApiKeySection` above) so that component — and the `useSettingsStore`
 * state/tests it already has — stays completely untouched by this
 * milestone; this one talks straight to the new `set_provider_api_key`/
 * `has_provider_api_key`/`clear_provider_api_key` commands instead.
 */
function ProviderApiKeySection({ provider, label, placeholder }: { provider: string; label: string; placeholder: string }) {
  const [hasKey, setHasKey] = useState<boolean | null>(null);
  const [input, setInput] = useState("");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!isTauriRuntime()) {
      setHasKey(false);
      return;
    }
    hasProviderApiKey(provider)
      .then(setHasKey)
      .catch(() => setHasKey(false));
  }, [provider]);

  const handleSave = async () => {
    setSaving(true);
    setError(null);
    try {
      await setProviderApiKey(provider, input);
      setHasKey(true);
      setInput("");
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSaving(false);
    }
  };

  const handleClear = async () => {
    setSaving(true);
    setError(null);
    try {
      await clearProviderApiKey(provider);
      setHasKey(false);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSaving(false);
    }
  };

  if (!isTauriRuntime()) {
    return null;
  }

  if (hasKey === null) {
    return <p className="text-xs text-muted-foreground">Checking keychain…</p>;
  }

  if (hasKey) {
    return (
      <div className="flex items-center justify-between gap-3 rounded-md border border-border bg-surface px-3 py-2.5">
        <div className="flex items-center gap-2">
          <Badge variant="success">Key configured</Badge>
          <span className="text-xs text-muted-foreground">{label} API key stored in the OS keychain.</span>
        </div>
        <Button variant="outline" size="sm" onClick={() => void handleClear()} disabled={saving}>
          Clear
        </Button>
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-2">
      <div className="flex gap-2">
        <Input
          type="password"
          placeholder={placeholder}
          value={input}
          onChange={(e) => setInput(e.target.value)}
          autoComplete="off"
          className="max-w-xs"
        />
        <Button onClick={() => void handleSave()} disabled={saving || input.trim().length === 0}>
          Save
        </Button>
      </div>
      <p className="text-xs text-muted-foreground">No {label} key set. Stored in the OS keychain — never in the database.</p>
      {error && <p className="text-xs text-destructive">{error}</p>}
    </div>
  );
}

/**
 * Read-only list of the known model configs (seeded with one Claude Sonnet 5
 * default by the M1 migration). Editing/adding models is a later milestone.
 */
function ModelConfigSection() {
  const [models, setModels] = useState<ModelConfig[] | null>(null);

  useEffect(() => {
    if (!isTauriRuntime()) {
      setModels([]);
      return;
    }
    listModelConfigs()
      .then(setModels)
      .catch(() => setModels([]));
  }, []);

  if (models === null) {
    return <p className="text-xs text-muted-foreground">Loading…</p>;
  }

  if (models.length === 0) {
    return <p className="text-xs text-muted-foreground">No model configs found.</p>;
  }

  return (
    <div className="flex flex-col gap-2">
      {models.map((model) => (
        <div
          key={model.id}
          className="flex items-center justify-between rounded-md border border-border bg-surface px-3 py-2.5"
        >
          <div className="flex items-center gap-2">
            <span className="text-sm font-medium text-foreground">{model.displayName}</span>
            <Badge variant="outline">{model.provider}</Badge>
            {model.isDefault && <Badge variant="outline">Default</Badge>}
          </div>
          <span className="text-xs text-muted-foreground">
            {model.modelId} · max {model.maxOutputTokens.toLocaleString()} output tokens
          </span>
        </div>
      ))}
      <p className="text-xs text-muted-foreground">
        Adding or editing models lands in a later milestone.
      </p>
    </div>
  );
}

/** The `settings` key M10's scheduler reads via `agent.max_parallel_agents` — see `orchestrator::scheduler`. */
const MAX_PARALLEL_AGENTS_KEY = "agent.max_parallel_agents";
/** Mirrors `orchestrator::scheduler::DEFAULT_MAX_PARALLEL_AGENTS` — used only if the value can't be read/parsed. */
const DEFAULT_MAX_PARALLEL_AGENTS = 3;

/**
 * M10: a real, backend-persisted setting (seeded to `3` by migration
 * `0004_max_parallel_agents.sql`, read once per mission start by
 * `orchestrator::scheduler::run_mission_inner`) controlling how many of a
 * running mission's ready tasks execute concurrently. Reads/writes through
 * the same generic `get_setting`/`set_setting` commands every other setting
 * in this app goes through — no new backend mechanism, just a new key.
 */
function MaxParallelAgentsSection() {
  const [savedValue, setSavedValue] = useState<number | null>(null);
  const [input, setInput] = useState("");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!isTauriRuntime()) {
      setSavedValue(DEFAULT_MAX_PARALLEL_AGENTS);
      setInput(String(DEFAULT_MAX_PARALLEL_AGENTS));
      return;
    }
    getSetting(MAX_PARALLEL_AGENTS_KEY)
      .then((raw) => {
        const parsed = raw !== null ? Number.parseInt(raw, 10) : NaN;
        const resolved = Number.isFinite(parsed) && parsed > 0 ? parsed : DEFAULT_MAX_PARALLEL_AGENTS;
        setSavedValue(resolved);
        setInput(String(resolved));
      })
      .catch(() => {
        setSavedValue(DEFAULT_MAX_PARALLEL_AGENTS);
        setInput(String(DEFAULT_MAX_PARALLEL_AGENTS));
      });
  }, []);

  const handleSave = async () => {
    const parsed = Number.parseInt(input, 10);
    if (!Number.isFinite(parsed) || parsed <= 0) {
      setError("Enter a whole number greater than 0.");
      return;
    }
    setSaving(true);
    setError(null);
    try {
      await setSetting(MAX_PARALLEL_AGENTS_KEY, String(parsed));
      setSavedValue(parsed);
      setInput(String(parsed));
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSaving(false);
    }
  };

  if (!isTauriRuntime()) {
    return <p className="text-xs text-muted-foreground">Execution settings require the desktop app runtime.</p>;
  }

  if (savedValue === null) {
    return <p className="text-xs text-muted-foreground">Loading…</p>;
  }

  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center gap-2">
        <Input
          type="number"
          min={1}
          step={1}
          value={input}
          onChange={(e) => setInput(e.target.value)}
          className="max-w-[6rem]"
        />
        <Button
          size="sm"
          onClick={() => void handleSave()}
          disabled={saving || input.trim() === String(savedValue)}
        >
          {saving ? "Saving…" : "Save"}
        </Button>
        <span className="text-xs text-muted-foreground">Currently {savedValue}</span>
      </div>
      <p className="text-xs text-muted-foreground">
        How many tasks a running mission (Tasks tab) executes at once, each in its own isolated git worktree. Takes
        effect the next time a mission is started — it doesn't change one already running.
      </p>
      {error && <p className="text-xs text-destructive">{error}</p>}
    </div>
  );
}

/** The four `model.role.*` settings keys (M20) — see `agent::model_resolution::ModelRole::setting_key`. */
const MODEL_ROLES: { key: string; label: string; description: string }[] = [
  { key: "model.role.orchestrator", label: "Orchestrator", description: "Mission planning (Tasks tab)" },
  { key: "model.role.coder", label: "Coder", description: "Normal agent runs" },
  { key: "model.role.reviewer", label: "Reviewer", description: "Code review" },
  { key: "model.role.utility", label: "Utility", description: "Project Brain analysis" },
];

/**
 * Phase 5 M20: per-role model assignment — four dropdowns (one per
 * `ModelRole`), each persisted as its own `model.role.<role>` setting (M4's
 * generic `get_setting`/`set_setting` commands, no new backend plumbing)
 * storing a `model_configs.id`. Leaving a dropdown on "Default" clears that
 * setting entirely (rather than persisting an empty string) so
 * `agent::model_resolution::resolve_model_config` falls through to the
 * global default exactly as if it had never been set — the "nothing breaks
 * for an existing install" guarantee this milestone promises.
 */
function ModelRoleSection() {
  const [models, setModels] = useState<ModelConfig[] | null>(null);
  const [assignments, setAssignments] = useState<Record<string, string>>({});
  const [savingKey, setSavingKey] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!isTauriRuntime()) {
      setModels([]);
      return;
    }
    listModelConfigs()
      .then(setModels)
      .catch(() => setModels([]));
  }, []);

  useEffect(() => {
    if (!isTauriRuntime()) return;
    Promise.all(MODEL_ROLES.map((role) => getSetting(role.key)))
      .then((values) => {
        const next: Record<string, string> = {};
        MODEL_ROLES.forEach((role, i) => {
          const value = values[i];
          if (value) next[role.key] = value;
        });
        setAssignments(next);
      })
      .catch(() => {});
  }, []);

  const handleChange = async (roleKey: string, modelConfigId: string) => {
    setSavingKey(roleKey);
    setError(null);
    try {
      if (modelConfigId === "") {
        // "Default" selected — clear the override rather than persist an
        // empty string, so resolution falls through to the global default.
        await setSetting(roleKey, "");
        setAssignments((prev) => {
          const next = { ...prev };
          delete next[roleKey];
          return next;
        });
      } else {
        await setSetting(roleKey, modelConfigId);
        setAssignments((prev) => ({ ...prev, [roleKey]: modelConfigId }));
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSavingKey(null);
    }
  };

  if (!isTauriRuntime()) {
    return <p className="text-xs text-muted-foreground">Model role assignment requires the desktop app runtime.</p>;
  }

  if (models === null) {
    return <p className="text-xs text-muted-foreground">Loading…</p>;
  }

  return (
    <div className="flex flex-col gap-3">
      {MODEL_ROLES.map((role) => (
        <div key={role.key} className="flex items-center justify-between gap-3">
          <div>
            <p className="text-sm font-medium text-foreground">{role.label}</p>
            <p className="text-xs text-muted-foreground">{role.description}</p>
          </div>
          <select
            className="h-9 min-w-[14rem] rounded-md border border-border bg-surface px-2 text-sm text-foreground"
            value={assignments[role.key] ?? ""}
            disabled={savingKey === role.key}
            onChange={(e) => void handleChange(role.key, e.target.value)}
          >
            <option value="">Default</option>
            {models.map((model) => (
              <option key={model.id} value={model.id}>
                {model.displayName} ({model.provider})
              </option>
            ))}
          </select>
        </div>
      ))}
      <p className="text-xs text-muted-foreground">
        Leaving a role on "Default" uses the global default model config above.
      </p>
      {error && <p className="text-xs text-destructive">{error}</p>}
    </div>
  );
}

export function Settings() {
  const theme = useSettingsStore((s) => s.theme);
  const setTheme = useSettingsStore((s) => s.setTheme);

  return (
    <div className="flex max-w-2xl flex-col gap-6">
      <h1 className="text-lg font-semibold text-foreground">Settings</h1>

      <SettingsSection title="Appearance" description="Choose how Forge Workspace looks.">
        <div className="flex gap-2">
          {themeOptions.map((option) => (
            <button
              key={option.value}
              type="button"
              onClick={() => setTheme(option.value)}
              className={cn(
                "flex items-center gap-2 rounded-md border px-3 py-2 text-sm font-medium transition-colors",
                theme === option.value
                  ? "border-primary bg-primary/10 text-foreground"
                  : "border-border text-muted-foreground hover:bg-surface-hover",
              )}
              aria-pressed={theme === option.value}
            >
              <option.icon className="h-4 w-4" />
              {option.label}
            </button>
          ))}
        </div>
      </SettingsSection>

      <Separator />

      <SettingsSection
        title="API Keys"
        description="Store provider API keys securely via the OS keychain."
      >
        <div className="flex flex-col gap-3">
          <ApiKeySection />
          <ProviderApiKeySection provider="openai" label="OpenAI" placeholder="sk-..." />
          <ProviderApiKeySection provider="google" label="Google" placeholder="AIza..." />
          <ProviderApiKeySection provider="openrouter" label="OpenRouter" placeholder="sk-or-..." />
        </div>
      </SettingsSection>

      <Separator />

      <SettingsSection
        title="Model Configuration"
        description="Choose default models and output limits for agent runs."
      >
        <ModelConfigSection />
      </SettingsSection>

      <Separator />

      <SettingsSection
        title="Model Roles"
        description="Assign a specific model per responsibility — orchestration, coding, review, and utility calls."
      >
        <ModelRoleSection />
      </SettingsSection>

      <Separator />

      <SettingsSection
        title="Execution"
        description="Controls how missions (Tasks tab) run their tasks."
      >
        <MaxParallelAgentsSection />
      </SettingsSection>
    </div>
  );
}
