import { useEffect } from "react";
import { useNavigate } from "react-router-dom";
import { Command } from "cmdk";
import {
  FolderGit2,
  FolderOpen,
  LayoutDashboard,
  Moon,
  PanelLeft,
  Settings as SettingsIcon,
  SquareTerminal,
  Sun,
} from "lucide-react";
import { useUIStore } from "@/stores/useUIStore";
import { useSettingsStore } from "@/stores/useSettingsStore";

/**
 * Cmd/Ctrl+K command palette. Only wires commands that are real in M1 (pure
 * client-side navigation + UI state toggles). Commands that need a Tauri
 * backend ("Open project", "Open terminal", "Search files") are shown as
 * visually-disabled rows so their presence is signposted without firing a
 * fake action.
 */
export function CommandPalette() {
  const open = useUIStore((s) => s.commandPaletteOpen);
  const setOpen = useUIStore((s) => s.setCommandPaletteOpen);
  const toggleSidebar = useUIStore((s) => s.toggleSidebar);
  const theme = useSettingsStore((s) => s.theme);
  const toggleTheme = useSettingsStore((s) => s.toggleTheme);
  const navigate = useNavigate();

  useEffect(() => {
    function onKeyDown(e: KeyboardEvent) {
      if (e.key === "k" && (e.metaKey || e.ctrlKey)) {
        e.preventDefault();
        setOpen(!open);
      }
      if (e.key === "Escape") {
        setOpen(false);
      }
    }
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [open, setOpen]);

  function runAndClose(fn: () => void) {
    fn();
    setOpen(false);
  }

  if (!open) return null;

  return (
    <div
      className="fixed inset-0 z-50 flex items-start justify-center bg-background/70 pt-[15vh] backdrop-blur-[2px]"
      onClick={() => setOpen(false)}
    >
      <Command
        className="w-full max-w-lg overflow-hidden rounded-lg border border-border bg-background-elevated shadow-xl animate-slide-down"
        onClick={(e) => e.stopPropagation()}
        label="Command palette"
      >
        <div className="flex items-center border-b border-border px-3">
          <Command.Input
            autoFocus
            placeholder="Type a command or search…"
            className="h-11 w-full bg-transparent text-sm text-foreground outline-none placeholder:text-subtle-foreground"
          />
        </div>
        <Command.List className="max-h-80 overflow-y-auto p-1.5">
          <Command.Empty className="px-3 py-6 text-center text-sm text-muted-foreground">
            No results found.
          </Command.Empty>

          <Command.Group
            heading="Navigate"
            className="px-2 pb-1 pt-2 text-[11px] font-semibold uppercase tracking-wider text-subtle-foreground [&_[cmdk-group-items]]:mt-1"
          >
            <PaletteItem
              icon={LayoutDashboard}
              label="Go to Dashboard"
              onSelect={() => runAndClose(() => navigate("/"))}
            />
            <PaletteItem
              icon={FolderGit2}
              label="Go to Projects"
              onSelect={() => runAndClose(() => navigate("/projects"))}
            />
            <PaletteItem
              icon={SettingsIcon}
              label="Go to Settings"
              onSelect={() => runAndClose(() => navigate("/settings"))}
            />
          </Command.Group>

          <Command.Group
            heading="Toggle"
            className="px-2 pb-1 pt-2 text-[11px] font-semibold uppercase tracking-wider text-subtle-foreground [&_[cmdk-group-items]]:mt-1"
          >
            <PaletteItem
              icon={theme === "dark" ? Sun : Moon}
              label={`Switch to ${theme === "dark" ? "light" : "dark"} theme`}
              onSelect={() => runAndClose(toggleTheme)}
            />
            <PaletteItem
              icon={PanelLeft}
              label="Toggle sidebar"
              onSelect={() => runAndClose(toggleSidebar)}
            />
          </Command.Group>

          <Command.Group
            heading="Coming soon"
            className="px-2 pb-1 pt-2 text-[11px] font-semibold uppercase tracking-wider text-subtle-foreground [&_[cmdk-group-items]]:mt-1"
          >
            <DisabledPaletteItem icon={FolderOpen} label="Open project" />
            <DisabledPaletteItem icon={SquareTerminal} label="Open terminal" />
          </Command.Group>
        </Command.List>
      </Command>
    </div>
  );
}

function PaletteItem({
  icon: Icon,
  label,
  onSelect,
}: {
  icon: typeof LayoutDashboard;
  label: string;
  onSelect: () => void;
}) {
  return (
    <Command.Item
      onSelect={onSelect}
      className="flex cursor-default items-center gap-2.5 rounded-md px-2.5 py-2 text-sm text-foreground data-[selected=true]:bg-surface-hover"
    >
      <Icon className="h-4 w-4 text-muted-foreground" />
      {label}
    </Command.Item>
  );
}

function DisabledPaletteItem({ icon: Icon, label }: { icon: typeof LayoutDashboard; label: string }) {
  return (
    <div className="flex cursor-not-allowed items-center gap-2.5 rounded-md px-2.5 py-2 text-sm text-subtle-foreground/60">
      <Icon className="h-4 w-4" />
      {label}
      <span className="ml-auto text-[10px]">Requires backend</span>
    </div>
  );
}
