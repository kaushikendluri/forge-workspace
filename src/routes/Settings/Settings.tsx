import { useEffect, useState, type ReactNode } from "react";
import { Moon, Sun } from "lucide-react";
import { cn } from "@/lib/utils";
import { useSettingsStore, type Theme } from "@/stores/useSettingsStore";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Separator } from "@/components/ui/separator";
import { getSetting, isTauriRuntime, listModelConfigs, setSetting } from "@/lib/tauri";
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
        <ApiKeySection />
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
        title="Execution"
        description="Controls how missions (Tasks tab) run their tasks."
      >
        <MaxParallelAgentsSection />
      </SettingsSection>
    </div>
  );
}
