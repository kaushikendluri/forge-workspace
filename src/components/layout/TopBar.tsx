import { useCallback, useEffect, useState } from "react";
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
import { isTauriRuntime, listNotifications, markNotificationRead, unreadNotificationCount } from "@/lib/tauri";
import type { Notification } from "@/types/db";

export function TopBar() {
  const activeProjectId = useProjectStore((s) => s.activeProjectId);
  const projects = useProjectStore((s) => s.projects);
  const activeBranch = useProjectStore((s) => s.activeBranch);
  const theme = useSettingsStore((s) => s.theme);
  const toggleTheme = useSettingsStore((s) => s.toggleTheme);
  const toggleCommandPalette = useUIStore((s) => s.toggleCommandPalette);

  const activeProject = projects.find((p) => p.id === activeProjectId);
  const isMac = typeof navigator !== "undefined" && navigator.platform.toLowerCase().includes("mac");

  // Real notifications from the `notifications` table (nothing produces any
  // yet — that starts with agent runs in M6 — so this is genuinely empty
  // rather than a decorative dead icon).
  const [notifications, setNotifications] = useState<Notification[]>([]);
  const [notificationsLoading, setNotificationsLoading] = useState(false);
  const [unreadCount, setUnreadCount] = useState(0);

  const refreshUnreadCount = useCallback(async () => {
    if (!isTauriRuntime()) return;
    try {
      setUnreadCount(await unreadNotificationCount(activeProjectId));
    } catch {
      setUnreadCount(0);
    }
  }, [activeProjectId]);

  useEffect(() => {
    void refreshUnreadCount();
  }, [refreshUnreadCount]);

  const handleNotificationsOpenChange = (open: boolean) => {
    if (!open || !isTauriRuntime()) return;
    setNotificationsLoading(true);
    listNotifications(activeProjectId)
      .then(setNotifications)
      .catch(() => setNotifications([]))
      .finally(() => setNotificationsLoading(false));
  };

  const handleMarkRead = async (id: string) => {
    try {
      await markNotificationRead(id);
      setNotifications((prev) => prev.map((n) => (n.id === id ? { ...n, isRead: true } : n)));
      void refreshUnreadCount();
    } catch {
      // Best-effort — leave the list as-is if the write fails.
    }
  };

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

        <DropdownMenu onOpenChange={handleNotificationsOpenChange}>
          <DropdownMenuTrigger asChild>
            <button
              type="button"
              className="relative flex h-8 w-8 items-center justify-center rounded-md text-muted-foreground hover:bg-surface-hover hover:text-foreground"
              aria-label="Notifications"
            >
              <Bell className="h-4 w-4" />
              {unreadCount > 0 && (
                <span className="absolute right-1.5 top-1.5 h-1.5 w-1.5 rounded-full bg-primary" />
              )}
            </button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="w-72 p-0">
            {notificationsLoading ? (
              <div className="px-4 py-8 text-center text-xs text-muted-foreground">Loading…</div>
            ) : notifications.length === 0 ? (
              <div className="flex flex-col items-center gap-1.5 px-4 py-8 text-center">
                <Bell className="h-4 w-4 text-subtle-foreground" />
                <p className="text-xs font-medium text-foreground">No notifications</p>
                <p className="text-xs text-muted-foreground">
                  You&apos;ll see agent run updates here once agents can run.
                </p>
              </div>
            ) : (
              <div className="flex max-h-80 flex-col overflow-y-auto py-1">
                {notifications.map((n) => (
                  <button
                    key={n.id}
                    type="button"
                    onClick={() => void handleMarkRead(n.id)}
                    className={cn(
                      "flex flex-col items-start gap-0.5 px-3 py-2 text-left text-xs hover:bg-surface-hover",
                      !n.isRead && "bg-surface",
                    )}
                  >
                    <span className="font-medium text-foreground">{n.title}</span>
                    {n.body && <span className="text-muted-foreground">{n.body}</span>}
                  </button>
                ))}
              </div>
            )}
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
