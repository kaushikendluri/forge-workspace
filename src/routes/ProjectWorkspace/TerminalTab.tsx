import { useState } from "react";
import { Plus, TerminalSquare, X } from "lucide-react";
import { TerminalView } from "@/components/terminal/TerminalView";
import { EmptyState } from "@/components/empty-states/EmptyState";
import { useProjectStore } from "@/stores/useProjectStore";
import { cn } from "@/lib/utils";

export interface TerminalTabProps {
  projectId: string;
}

function makeTabKey(): string {
  return typeof crypto !== "undefined" && "randomUUID" in crypto
    ? crypto.randomUUID()
    : `tab-${Date.now()}-${Math.random()}`;
}

/**
 * Real component for the Terminal tab: one or more real PTY-backed
 * terminals (each a `TerminalView`), with a tab bar to open/close/switch
 * between them. Tabs are keyed by a client-generated id rather than the
 * backend terminal id — the backend id isn't known until `terminal_spawn`
 * resolves inside each `TerminalView`. Inactive tabs stay mounted (hidden,
 * not unmounted) so their shells keep running in the background.
 */
export function TerminalTab({ projectId }: TerminalTabProps) {
  const rootPath = useProjectStore((s) => s.projects.find((p) => p.id === projectId)?.rootPath);
  const [tabKeys, setTabKeys] = useState<string[]>(() => [makeTabKey()]);
  const [activeTabKey, setActiveTabKey] = useState<string>(() => tabKeys[0]);

  if (!rootPath) {
    return (
      <EmptyState icon={TerminalSquare} title="No project open" description="Open a project to use its terminal." />
    );
  }

  const handleNewTab = () => {
    const key = makeTabKey();
    setTabKeys((keys) => [...keys, key]);
    setActiveTabKey(key);
  };

  const handleCloseTab = (key: string) => {
    const next = tabKeys.filter((k) => k !== key);
    setTabKeys(next);
    if (activeTabKey === key) {
      setActiveTabKey(next[next.length - 1] ?? "");
    }
  };

  return (
    <div className="flex flex-1 flex-col gap-2 overflow-hidden">
      <div className="flex items-center gap-1 border-b border-border pb-1">
        {tabKeys.map((key, index) => (
          <div
            key={key}
            className={cn(
              "flex items-center gap-1.5 rounded-t-md border border-b-0 border-transparent px-2 py-1 text-xs text-muted-foreground",
              activeTabKey === key && "border-border bg-surface text-foreground",
            )}
          >
            <button type="button" onClick={() => setActiveTabKey(key)} className="hover:text-foreground">
              Terminal {index + 1}
            </button>
            <button
              type="button"
              onClick={() => handleCloseTab(key)}
              title="Close terminal"
              className="rounded p-0.5 hover:bg-surface-hover"
            >
              <X className="h-3 w-3" />
            </button>
          </div>
        ))}
        <button
          type="button"
          onClick={handleNewTab}
          title="New terminal"
          className="ml-1 flex h-6 w-6 items-center justify-center rounded text-muted-foreground hover:bg-surface-hover hover:text-foreground"
        >
          <Plus className="h-3.5 w-3.5" />
        </button>
      </div>
      <div className="relative flex-1 overflow-hidden">
        {tabKeys.length === 0 && (
          <EmptyState icon={TerminalSquare} title="No terminals open" description="Open a new terminal to get started." />
        )}
        {tabKeys.map((key) => (
          <TerminalView
            key={key}
            cwd={rootPath}
            projectId={projectId}
            hidden={key !== activeTabKey}
            className="absolute inset-0"
          />
        ))}
      </div>
    </div>
  );
}
