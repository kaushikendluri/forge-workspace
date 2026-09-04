import { useEffect, useRef } from "react";
import * as monaco from "monaco-editor";
import editorWorker from "monaco-editor/esm/vs/editor/editor.worker?worker";
import { FileDiff } from "lucide-react";
import { cn } from "@/lib/utils";

// Monaco needs its web workers wired up manually under Vite (no CDN, no
// extra bundler plugin). Only the base editor worker is needed for diffing
// plain text/code — language-specific workers (ts, css, json) can be added
// here later if syntax-aware features are needed.
// eslint-disable-next-line @typescript-eslint/no-explicit-any
(self as any).MonacoEnvironment = {
  getWorker() {
    return new editorWorker();
  },
};

export interface MonacoDiffViewerProps {
  /** Original ("before") file content. Omit along with `modified` to show the empty state. */
  original?: string;
  /** Modified ("after") file content. */
  modified?: string;
  language?: string;
  path?: string;
  className?: string;
}

/**
 * Wraps monaco-editor's diff editor. Not wired to any real diff source yet
 * (that lands with the Changes tab backend in a later milestone) — renders
 * an empty-state message whenever `original`/`modified` aren't both passed.
 */
export function MonacoDiffViewer({
  original,
  modified,
  language = "plaintext",
  path,
  className,
}: MonacoDiffViewerProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const diffEditorRef = useRef<monaco.editor.IStandaloneDiffEditor | null>(null);

  const hasDiff = original !== undefined && modified !== undefined;

  useEffect(() => {
    if (!hasDiff || !containerRef.current) return;

    const isDark = !document.documentElement.classList.contains("light");
    const diffEditor = monaco.editor.createDiffEditor(containerRef.current, {
      automaticLayout: true,
      readOnly: true,
      renderSideBySide: true,
      theme: isDark ? "vs-dark" : "vs",
      minimap: { enabled: false },
      fontFamily: '"JetBrains Mono", "SF Mono", "Cascadia Code", monospace',
      fontSize: 13,
    });
    diffEditorRef.current = diffEditor;

    const originalModel = monaco.editor.createModel(original ?? "", language, path ? monaco.Uri.parse(`${path}.original`) : undefined);
    const modifiedModel = monaco.editor.createModel(modified ?? "", language, path ? monaco.Uri.parse(path) : undefined);

    diffEditor.setModel({ original: originalModel, modified: modifiedModel });

    return () => {
      diffEditor.dispose();
      originalModel.dispose();
      modifiedModel.dispose();
      diffEditorRef.current = null;
    };
  }, [hasDiff, original, modified, language, path]);

  if (!hasDiff) {
    return (
      <div
        className={cn(
          "flex flex-1 flex-col items-center justify-center gap-2 rounded-lg border border-dashed border-border py-16 text-center",
          className,
        )}
      >
        <FileDiff className="h-5 w-5 text-subtle-foreground" />
        <p className="text-sm text-muted-foreground">No diff to display</p>
      </div>
    );
  }

  return <div ref={containerRef} className={cn("h-full min-h-[240px] w-full", className)} />;
}
