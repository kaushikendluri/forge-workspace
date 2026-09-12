import { useEffect, useState } from "react";
import { File, Files, Folder, FolderUp } from "lucide-react";
import { EmptyState } from "@/components/empty-states/EmptyState";
import { useProjectStore } from "@/stores/useProjectStore";
import { listDirectory, readFilePreview } from "@/lib/tauri";
import type { DirEntryDto, FilePreviewDto } from "@/types/db";
import { cn } from "@/lib/utils";

export interface FilesTabProps {
  projectId: string;
}

/** Read-only preview cap — plenty for source files; large/binary files are
 * reported as truncated/binary by the backend rather than fully read. */
const MAX_PREVIEW_BYTES = 1_000_000;

function errorMessage(err: unknown): string {
  if (typeof err === "string") return err;
  if (err instanceof Error) return err.message;
  return String(err);
}

function formatSize(bytes: number | null): string {
  if (bytes === null) return "";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

/**
 * Real component for the Files tab: a directory listing rooted at the
 * active project (`list_directory`), with click-to-preview for files
 * (`read_file_preview`). Not a full editor — read-only text preview, with
 * honest binary/truncated/error states instead of pretending to show
 * something it can't.
 */
export function FilesTab({ projectId }: FilesTabProps) {
  const rootPath = useProjectStore((s) => s.projects.find((p) => p.id === projectId)?.rootPath);

  const [currentDir, setCurrentDir] = useState<string | null>(null);
  const [entries, setEntries] = useState<DirEntryDto[]>([]);
  const [listError, setListError] = useState<string | null>(null);

  const [selectedFile, setSelectedFile] = useState<DirEntryDto | null>(null);
  const [preview, setPreview] = useState<FilePreviewDto | null>(null);
  const [previewError, setPreviewError] = useState<string | null>(null);
  const [isPreviewLoading, setIsPreviewLoading] = useState(false);

  useEffect(() => {
    setCurrentDir(rootPath ?? null);
    setSelectedFile(null);
    setPreview(null);
  }, [rootPath]);

  useEffect(() => {
    if (!currentDir) return;
    let cancelled = false;
    setListError(null);
    listDirectory(currentDir)
      .then((result) => {
        if (!cancelled) setEntries(result);
      })
      .catch((err) => {
        if (!cancelled) {
          setListError(errorMessage(err));
          setEntries([]);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [currentDir]);

  useEffect(() => {
    if (!selectedFile) {
      setPreview(null);
      setPreviewError(null);
      return;
    }
    let cancelled = false;
    setIsPreviewLoading(true);
    setPreviewError(null);
    readFilePreview(selectedFile.path, MAX_PREVIEW_BYTES)
      .then((result) => {
        if (!cancelled) setPreview(result);
      })
      .catch((err) => {
        if (cancelled) return;
        setPreview(null);
        setPreviewError(errorMessage(err));
      })
      .finally(() => {
        if (!cancelled) setIsPreviewLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [selectedFile]);

  if (!rootPath || !currentDir) {
    return <EmptyState icon={Files} title="No project open" description="Open a project to browse its files." />;
  }

  const canGoUp = currentDir.length > rootPath.length;

  const handleUp = () => {
    // Paths may use either separator depending on how they were joined
    // upstream (Windows tolerates both) — split on either so "up a level"
    // works regardless.
    const parent = currentDir.split(/[\\/]/).slice(0, -1).join("/");
    setCurrentDir(parent.length >= rootPath.length ? parent : rootPath);
    setSelectedFile(null);
  };

  return (
    <div className="flex flex-1 gap-4 overflow-hidden">
      <div className="flex w-72 shrink-0 flex-col gap-2 overflow-hidden">
        <div className="flex items-center gap-2">
          <h2 className="min-w-0 flex-1 truncate text-sm font-medium text-foreground" title={currentDir}>
            {currentDir === rootPath ? "Files" : currentDir.slice(rootPath.length + 1)}
          </h2>
          {canGoUp && (
            <button
              type="button"
              onClick={handleUp}
              title="Up a level"
              className="flex h-6 w-6 shrink-0 items-center justify-center rounded text-muted-foreground hover:bg-surface-hover hover:text-foreground"
            >
              <FolderUp className="h-3.5 w-3.5" />
            </button>
          )}
        </div>
        {listError && (
          <div className="rounded-md border border-destructive/40 bg-destructive/10 px-2 py-1.5 text-xs text-destructive">
            {listError}
          </div>
        )}
        <div className="flex flex-1 flex-col gap-0.5 overflow-y-auto">
          {!listError && entries.length === 0 && (
            <p className="px-2 py-1 text-xs text-muted-foreground">Empty directory.</p>
          )}
          {entries.map((entry) => (
            <button
              key={entry.path}
              type="button"
              title={entry.path}
              onClick={() => (entry.isDir ? setCurrentDir(entry.path) : setSelectedFile(entry))}
              className={cn(
                "flex items-center gap-2 rounded px-2 py-1 text-left text-xs text-foreground hover:bg-surface-hover",
                !entry.isDir && selectedFile?.path === entry.path && "bg-surface-hover",
              )}
            >
              {entry.isDir ? (
                <Folder className="h-3.5 w-3.5 shrink-0 text-subtle-foreground" />
              ) : (
                <File className="h-3.5 w-3.5 shrink-0 text-subtle-foreground" />
              )}
              <span className="min-w-0 flex-1 truncate">{entry.name}</span>
              {!entry.isDir && <span className="shrink-0 text-subtle-foreground">{formatSize(entry.sizeBytes)}</span>}
            </button>
          ))}
        </div>
      </div>
      <div className="flex flex-1 flex-col gap-2 overflow-hidden">
        {!selectedFile ? (
          <EmptyState icon={File} title="No file selected" description="Select a file to preview its contents." />
        ) : (
          <>
            <div className="flex items-center justify-between gap-2">
              <h3 className="min-w-0 flex-1 truncate text-sm font-medium text-foreground" title={selectedFile.path}>
                {selectedFile.name}
              </h3>
              {preview?.truncated && (
                <span className="shrink-0 text-xs text-subtle-foreground">Preview truncated</span>
              )}
            </div>
            {previewError && (
              <div className="rounded-md border border-destructive/40 bg-destructive/10 px-2 py-1.5 text-xs text-destructive">
                {previewError}
              </div>
            )}
            {isPreviewLoading && <p className="text-xs text-muted-foreground">Loading preview…</p>}
            {preview?.isBinary && <p className="text-xs text-muted-foreground">Binary file — no preview available.</p>}
            {preview && !preview.isBinary && (
              <pre className="flex-1 overflow-auto whitespace-pre-wrap rounded-lg border border-border bg-background-elevated p-3 font-mono text-xs text-foreground">
                {preview.content}
              </pre>
            )}
          </>
        )}
      </div>
    </div>
  );
}
