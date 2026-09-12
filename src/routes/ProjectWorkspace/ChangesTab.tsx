import { useEffect, useMemo, useState } from "react";
import { FileDiff } from "lucide-react";
import { MonacoDiffViewer } from "@/components/diff/MonacoDiffViewer";
import { EmptyState } from "@/components/empty-states/EmptyState";
import { useProjectStore } from "@/stores/useProjectStore";
import { gitDiffFile, gitStatus } from "@/lib/tauri";
import type { GitFileDiff, GitStatus } from "@/types/db";
import { cn } from "@/lib/utils";

export interface ChangesTabProps {
  projectId: string;
}

type ChangeGroup = "staged" | "unstaged" | "untracked";

interface ChangedFile {
  path: string;
  statusCode: string;
  group: ChangeGroup;
}

function errorMessage(err: unknown): string {
  if (typeof err === "string") return err;
  if (err instanceof Error) return err.message;
  return String(err);
}

/** Flattens `GitStatus` into one deduplicated, sorted list — a file staged
 * and then further modified only needs one row here (the diff viewer always
 * compares HEAD to the working tree, so staged-vs-unstaged doesn't change
 * what's shown). */
function collectChangedFiles(status: GitStatus): ChangedFile[] {
  const byPath = new Map<string, ChangedFile>();
  for (const entry of status.staged) {
    byPath.set(entry.path, { path: entry.path, statusCode: entry.statusCode, group: "staged" });
  }
  for (const entry of status.unstaged) {
    if (!byPath.has(entry.path)) {
      byPath.set(entry.path, { path: entry.path, statusCode: entry.statusCode, group: "unstaged" });
    }
  }
  for (const path of status.untracked) {
    if (!byPath.has(path)) {
      byPath.set(path, { path, statusCode: "?", group: "untracked" });
    }
  }
  return Array.from(byPath.values()).sort((a, b) => a.path.localeCompare(b.path));
}

/**
 * Real component for the Changes tab: the changed-files list comes from
 * `git_status`, and clicking a file fetches its before/after text via
 * `git_diff_file` and feeds it into `MonacoDiffViewer`.
 */
export function ChangesTab({ projectId }: ChangesTabProps) {
  const rootPath = useProjectStore((s) => s.projects.find((p) => p.id === projectId)?.rootPath);

  const [status, setStatus] = useState<GitStatus | null>(null);
  const [statusError, setStatusError] = useState<string | null>(null);
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  const [diff, setDiff] = useState<GitFileDiff | null>(null);
  const [diffError, setDiffError] = useState<string | null>(null);
  const [isDiffLoading, setIsDiffLoading] = useState(false);

  useEffect(() => {
    if (!rootPath) return;
    let cancelled = false;
    setStatusError(null);
    gitStatus(rootPath)
      .then((s) => {
        if (!cancelled) setStatus(s);
      })
      .catch((err) => {
        if (!cancelled) setStatusError(errorMessage(err));
      });
    return () => {
      cancelled = true;
    };
  }, [rootPath]);

  const files = useMemo(() => (status ? collectChangedFiles(status) : []), [status]);

  useEffect(() => {
    if (!rootPath || !selectedPath) {
      setDiff(null);
      setDiffError(null);
      return;
    }
    let cancelled = false;
    setIsDiffLoading(true);
    setDiffError(null);
    gitDiffFile(rootPath, selectedPath)
      .then((d) => {
        if (!cancelled) setDiff(d);
      })
      .catch((err) => {
        if (cancelled) return;
        setDiff(null);
        setDiffError(errorMessage(err));
      })
      .finally(() => {
        if (!cancelled) setIsDiffLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [rootPath, selectedPath]);

  if (!rootPath) {
    return (
      <EmptyState icon={FileDiff} title="No project open" description="Open a project to see its changes." />
    );
  }

  return (
    <div className="flex flex-1 gap-4 overflow-hidden">
      <div className="flex w-64 shrink-0 flex-col gap-2 overflow-hidden">
        <h2 className="text-sm font-medium text-foreground">Changes</h2>
        {statusError && (
          <div className="rounded-md border border-destructive/40 bg-destructive/10 px-2 py-1.5 text-xs text-destructive">
            {statusError}
          </div>
        )}
        <div className="flex flex-1 flex-col gap-0.5 overflow-y-auto">
          {!statusError && files.length === 0 && (
            <p className="px-2 py-1 text-xs text-muted-foreground">No changes in the working tree.</p>
          )}
          {files.map((file) => (
            <button
              key={`${file.group}-${file.path}`}
              type="button"
              title={file.path}
              onClick={() => setSelectedPath(file.path)}
              className={cn(
                "flex items-center gap-2 rounded px-2 py-1 text-left text-xs text-foreground hover:bg-surface-hover",
                selectedPath === file.path && "bg-surface-hover",
              )}
            >
              <span className="w-4 shrink-0 font-mono text-subtle-foreground">{file.statusCode}</span>
              <span className="truncate">{file.path}</span>
            </button>
          ))}
        </div>
      </div>
      <div className="flex flex-1 flex-col gap-2 overflow-hidden">
        {diffError && (
          <div className="rounded-md border border-destructive/40 bg-destructive/10 px-2 py-1.5 text-xs text-destructive">
            {diffError}
          </div>
        )}
        {isDiffLoading && <p className="text-xs text-muted-foreground">Loading diff…</p>}
        <MonacoDiffViewer
          original={diff?.original}
          modified={diff?.modified}
          path={selectedPath ?? undefined}
          className="flex-1"
        />
      </div>
    </div>
  );
}
