import { useEffect, useState } from "react";
import { FlaskConical } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { EmptyState } from "@/components/empty-states/EmptyState";
import { useProjectStore } from "@/stores/useProjectStore";
import { getProjectCommandSettings, listTestRuns, runTestSuite, setProjectCommandSetting } from "@/lib/tauri";
import type { CommandSettingDto, TestRun, TestRunKind, TestRunStatus } from "@/types/db";
import { cn } from "@/lib/utils";

export interface TestingTabProps {
  projectId: string;
}

function errorMessage(err: unknown): string {
  if (typeof err === "string") return err;
  if (err instanceof Error) return err.message;
  return String(err);
}

const KIND_LABEL: Record<TestRunKind, string> = { test: "Test", lint: "Lint", build: "Build" };
const KIND_RUN_LABEL: Record<TestRunKind, string> = { test: "Run tests", lint: "Run lint", build: "Run build" };

const RUN_STATUS_BADGE: Record<TestRunStatus, { label: string; variant: "default" | "success" | "destructive" }> = {
  running: { label: "Running", variant: "default" },
  success: { label: "Passed", variant: "success" },
  failure: { label: "Failed", variant: "destructive" },
};

/** Honest three-state badge for a command's provenance — never just a blank field. */
function SourceBadge({ setting }: { setting: CommandSettingDto }) {
  if (!setting.value) {
    return <Badge variant="outline">Not detected</Badge>;
  }
  if (setting.source === "detected") {
    return <Badge variant="secondary">Auto-detected</Badge>;
  }
  if (setting.source === "user") {
    return <Badge variant="outline">Custom</Badge>;
  }
  return <Badge variant="outline">Configured</Badge>;
}

function formatTimestamp(iso: string): string {
  return new Date(iso).toLocaleString();
}

/**
 * One kind's (test/lint/build) settings + manual run panel: shows whether
 * the configured command was auto-detected from the project's own files
 * (`project_detect::detect_commands`) or set by hand, lets the user edit it,
 * runs it via the real `run_test_suite` command (which shells out through
 * the same process-execution logic the agent's own `run_tests`/`run_linter`/
 * `run_build` tools use), and lists real run history.
 */
function CommandPanel({ projectId, kind }: { projectId: string; kind: TestRunKind }) {
  const [setting, setSetting] = useState<CommandSettingDto | null>(null);
  const [input, setInput] = useState("");
  const [isEditing, setIsEditing] = useState(false);
  const [isSaving, setIsSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);

  const [history, setHistory] = useState<TestRun[] | null>(null);
  const [historyError, setHistoryError] = useState<string | null>(null);
  const [selectedRun, setSelectedRun] = useState<TestRun | null>(null);
  const [isRunning, setIsRunning] = useState(false);
  const [runError, setRunError] = useState<string | null>(null);

  const loadHistory = () => {
    listTestRuns(projectId, kind)
      .then((runs) => {
        setHistory(runs);
        setHistoryError(null);
        setSelectedRun((current) => current ?? runs[0] ?? null);
      })
      .catch((err) => setHistoryError(errorMessage(err)));
  };

  useEffect(() => {
    let cancelled = false;
    getProjectCommandSettings(projectId)
      .then((settings) => {
        if (cancelled) return;
        const s = kind === "test" ? settings.testCommand : kind === "lint" ? settings.lintCommand : settings.buildCommand;
        setSetting(s);
        setInput(s.value ?? "");
      })
      .catch(() => {
        if (!cancelled) setSetting({ value: null, source: null });
      });
    return () => {
      cancelled = true;
    };
  }, [projectId, kind]);

  useEffect(() => {
    setHistory(null);
    setSelectedRun(null);
    loadHistory();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [projectId, kind]);

  const handleSave = async () => {
    if (input.trim().length === 0) {
      setSaveError("Command cannot be empty.");
      return;
    }
    setIsSaving(true);
    setSaveError(null);
    try {
      await setProjectCommandSetting(projectId, kind, input.trim());
      setSetting({ value: input.trim(), source: "user" });
      setIsEditing(false);
    } catch (err) {
      setSaveError(errorMessage(err));
    } finally {
      setIsSaving(false);
    }
  };

  const handleRun = async () => {
    setIsRunning(true);
    setRunError(null);
    try {
      const run = await runTestSuite(projectId, kind);
      setSelectedRun(run);
      loadHistory();
    } catch (err) {
      setRunError(errorMessage(err));
    } finally {
      setIsRunning(false);
    }
  };

  if (!setting) {
    return <p className="text-xs text-muted-foreground">Loading…</p>;
  }

  const canRun = Boolean(setting.value);

  return (
    <div className="flex flex-col gap-2 rounded-md border border-border p-3">
      <div className="flex items-center justify-between gap-2">
        <div className="flex items-center gap-2">
          <h3 className="text-sm font-medium text-foreground">{KIND_LABEL[kind]}</h3>
          <SourceBadge setting={setting} />
        </div>
        <Button size="sm" onClick={() => void handleRun()} disabled={!canRun || isRunning} title={canRun ? undefined : "Configure a command below first"}>
          {isRunning ? "Running…" : KIND_RUN_LABEL[kind]}
        </Button>
      </div>

      {isEditing ? (
        <div className="flex flex-col gap-1.5">
          <div className="flex gap-2">
            <Input value={input} onChange={(e) => setInput(e.target.value)} placeholder={`e.g. npm test`} className="font-mono text-xs" />
            <Button size="sm" onClick={() => void handleSave()} disabled={isSaving}>
              {isSaving ? "Saving…" : "Save"}
            </Button>
            <Button
              size="sm"
              variant="outline"
              onClick={() => {
                setIsEditing(false);
                setInput(setting.value ?? "");
                setSaveError(null);
              }}
            >
              Cancel
            </Button>
          </div>
          {saveError && <p className="text-xs text-destructive">{saveError}</p>}
        </div>
      ) : (
        <div className="flex items-center justify-between gap-2">
          {setting.value ? (
            <code className="truncate rounded bg-surface px-2 py-1 text-xs text-foreground">{setting.value}</code>
          ) : (
            <p className="text-xs text-muted-foreground">Not detected — no {KIND_LABEL[kind].toLowerCase()} command found for this project.</p>
          )}
          <Button size="sm" variant="outline" onClick={() => setIsEditing(true)}>
            {setting.value ? "Edit" : "Set command"}
          </Button>
        </div>
      )}

      {runError && (
        <div className="rounded-md border border-destructive/40 bg-destructive/10 px-2 py-1.5 text-xs text-destructive">{runError}</div>
      )}

      <div className="flex gap-3">
        <div className="flex w-40 shrink-0 flex-col gap-0.5">
          <span className="text-[11px] font-medium uppercase tracking-wide text-subtle-foreground">History</span>
          {historyError && <p className="text-xs text-destructive">{historyError}</p>}
          {history !== null && history.length === 0 && <p className="text-xs text-muted-foreground">No runs yet.</p>}
          <div className="flex max-h-40 flex-col gap-0.5 overflow-y-auto">
            {history?.map((run) => (
              <button
                key={run.id}
                type="button"
                onClick={() => setSelectedRun(run)}
                className={cn(
                  "flex items-center justify-between gap-1.5 rounded px-1.5 py-1 text-left text-[11px] hover:bg-surface-hover",
                  selectedRun?.id === run.id && "bg-surface-hover",
                )}
              >
                <span className="text-subtle-foreground">{formatTimestamp(run.startedAt)}</span>
                <Badge variant={RUN_STATUS_BADGE[run.status].variant}>{RUN_STATUS_BADGE[run.status].label}</Badge>
              </button>
            ))}
          </div>
        </div>
        <div className="flex flex-1 flex-col gap-1 overflow-hidden">
          {selectedRun ? (
            <>
              <div className="flex items-center gap-2 text-[11px] text-subtle-foreground">
                <code className="truncate">{selectedRun.command}</code>
                <Badge variant={RUN_STATUS_BADGE[selectedRun.status].variant}>{RUN_STATUS_BADGE[selectedRun.status].label}</Badge>
                {selectedRun.exitCode !== null && <span>exit {selectedRun.exitCode}</span>}
              </div>
              <pre className="max-h-48 overflow-auto whitespace-pre-wrap break-words rounded bg-surface p-2 text-[11px] text-muted-foreground">
                {selectedRun.output ?? "(no output yet)"}
              </pre>
            </>
          ) : (
            <p className="text-xs text-muted-foreground">No run selected.</p>
          )}
        </div>
      </div>
    </div>
  );
}

/**
 * Project-level Testing tab (M12): detected/configured test/lint/build
 * commands for the active project, each independently editable, runnable on
 * demand, and with real persisted run history — see `CommandPanel` above.
 */
export function TestingTab({ projectId }: TestingTabProps) {
  const rootPath = useProjectStore((s) => s.projects.find((p) => p.id === projectId)?.rootPath);

  if (!rootPath) {
    return <EmptyState icon={FlaskConical} title="No project open" description="Open a project to run its tests." />;
  }

  return (
    <div className="flex flex-1 flex-col gap-3 overflow-y-auto">
      <h2 className="text-sm font-medium text-foreground">Testing</h2>
      <CommandPanel projectId={projectId} kind="test" />
      <CommandPanel projectId={projectId} kind="lint" />
      <CommandPanel projectId={projectId} kind="build" />
    </div>
  );
}
