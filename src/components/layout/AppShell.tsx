import { useEffect } from "react";
import { Outlet } from "react-router-dom";
import { Panel, PanelGroup, PanelResizeHandle } from "react-resizable-panels";
import { Sidebar } from "./Sidebar";
import { TopBar } from "./TopBar";
import { CommandPalette } from "./CommandPalette";
import { TooltipProvider } from "@/components/ui/tooltip";
import { useSettingsStore } from "@/stores/useSettingsStore";

/**
 * Top-level app frame: collapsible sidebar + resizable content area, topped
 * by the TopBar, with the command palette mounted globally. Sidebar width
 * itself is owned by <Sidebar />, driven by useUIStore's sidebarCollapsed.
 */
export function AppShell() {
  const loadThemeFromBackend = useSettingsStore((s) => s.loadThemeFromBackend);

  // Reconcile the instant, localStorage-sourced theme with the backend's
  // `ui.theme` setting once, at app startup — the backend wins if it
  // disagrees (SQLite is the real state).
  useEffect(() => {
    void loadThemeFromBackend();
  }, [loadThemeFromBackend]);

  return (
    <TooltipProvider delayDuration={300}>
      <div className="flex h-screen w-screen overflow-hidden bg-background text-foreground">
        <Sidebar />
        <PanelGroup direction="horizontal" className="flex-1" autoSaveId="forge-main-panels">
          <Panel
            id="main-content"
            order={1}
            minSize={40}
            className="flex min-w-0 flex-1 flex-col"
          >
            <TopBar />
            <main className="flex min-h-0 flex-1 flex-col overflow-auto p-4">
              <Outlet />
            </main>
          </Panel>
          {/*
            A second, currently-empty panel + handle keeps the resizable
            layout genuinely functional (drag to resize) even though nothing
            occupies the secondary pane until a later milestone (e.g. an
            inspector/detail rail). Collapsed to 0 by default via minSize=0.
          */}
          <PanelResizeHandle
            className="hidden w-px bg-border transition-colors hover:bg-primary/50 data-[resize-handle-active]:bg-primary md:block"
          />
          <Panel
            id="secondary"
            order={2}
            defaultSize={0}
            minSize={0}
            maxSize={40}
            className="hidden md:block"
          />
        </PanelGroup>
      </div>
      <CommandPalette />
    </TooltipProvider>
  );
}
