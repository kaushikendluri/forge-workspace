import { Bell, GitBranch, Moon, Search, Sun } from "lucide-react";
import { cn } from "@/lib/utils";
import { useProjectStore } from "@/stores/useProjectStore";
import { useSettingsStore } from "@/stores/useSettingsStore";
import { useUIStore } from "@/stores/useUIStore";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";

export function TopBar() {
  const activeProjectId = useProjectStore((s) => s.activeProjectId);
  const projects = useProjectStore((s) => s.projects);
  const activeBranch = useProjectStore((s) => s.activeBranch);
  const theme = useSettingsStore((s) => s.theme);
  const toggleTheme = useSettingsStore((s) => s.toggleTheme);
  const toggleCommandPalette = useUIStore((s) => s.toggleCommandPalette);

  const activeProject = projects.find((p) => p.id === activeProjectId);
  const isMac = typeof navigator !== "undefined" && navigator.platform.toLowerCase().includes("mac");

  return (
    <header className="flex h-12 shrink-0 items-center justify-between border-b border-border bg-background-elevated px-3">
      <div className="flex min-w-0 items-center gap-3">
        <span className="truncate text-sm font-medium text-foreground">
          {activeProject ? activeProject.name : "No project open"}
        </span>
        {activeProject && (
          <span className="flex items-center gap-1 text-xs text-subtle-foreground">
            <GitBranch className="h-3.5 w-3.5" />
            {activeBranch ?? activeProject.defaultBranch}
          </span>
        )}
      </div>

      <div className="flex items-center gap-1.5">
        <Tooltip delayDuration={300}>
          <TooltipTrigger asChild>
            <button
              type="button"
              onClick={toggleCommandPalette}
              className="flex h-8 items-center gap-2 rounded-md border border-border bg-background px-2.5 text-xs text-muted-foreground hover:bg-surface-hover"
            >
              <Search className="h-3.5 w-3.5" />
              <span className="hidden sm:inline">Search</span>
              <kbd className="ml-1 hidden rounded border border-border bg-surface px-1.5 py-0.5 font-mono text-[10px] sm:inline">
                {isMac ? "⌘K" : "Ctrl+K"}
              </kbd>
            </button>
          </TooltipTrigger>
          <TooltipContent>Open command palette</TooltipContent>
        </Tooltip>

        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <button
              type="button"
              className="flex h-8 w-8 items-center justify-center rounded-md text-muted-foreground hover:bg-surface-hover hover:text-foreground"
              aria-label="Notifications"
            >
              <Bell className="h-4 w-4" />
            </button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="w-72 p-0">
            <div className="flex flex-col items-center gap-1.5 px-4 py-8 text-center">
              <Bell className="h-4 w-4 text-subtle-foreground" />
              <p className="text-xs font-medium text-foreground">No notifications</p>
              <p className="text-xs text-muted-foreground">
                You&apos;ll see agent run updates here once agents can run.
              </p>
            </div>
          </DropdownMenuContent>
        </DropdownMenu>

        <Tooltip delayDuration={300}>
          <TooltipTrigger asChild>
            <button
              type="button"
              onClick={toggleTheme}
              className="flex h-8 w-8 items-center justify-center rounded-md text-muted-foreground hover:bg-surface-hover hover:text-foreground"
              aria-label="Toggle theme"
            >
              {theme === "dark" ? (
                <Moon className={cn("h-4 w-4")} />
              ) : (
                <Sun className={cn("h-4 w-4")} />
              )}
            </button>
          </TooltipTrigger>
          <TooltipContent>Switch to {theme === "dark" ? "light" : "dark"} theme</TooltipContent>
        </Tooltip>
      </div>
    </header>
  );
}
