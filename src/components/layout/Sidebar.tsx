import { NavLink } from "react-router-dom";
import type { LucideIcon } from "lucide-react";
import {
  Activity,
  Bot,
  ChevronsLeft,
  ChevronsRight,
  FlaskConical,
  Files,
  FolderGit2,
  GitCompare,
  LayoutGrid,
  ListChecks,
  Settings as SettingsIcon,
  SquareTerminal,
  Sparkles,
  BrainCircuit,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { useUIStore } from "@/stores/useUIStore";
import { useProjectStore } from "@/stores/useProjectStore";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";

interface NavItem {
  label: string;
  to: string;
  icon: LucideIcon;
}

interface DisabledNavItem {
  label: string;
  icon: LucideIcon;
  reason: string;
}

const workspaceItems: NavItem[] = [
  { label: "Projects", to: "/projects", icon: FolderGit2 },
  { label: "Agents", to: "/agents", icon: Bot },
  { label: "Tasks", to: "/tasks", icon: ListChecks },
  { label: "Activity", to: "/activity", icon: Activity },
];

const skillsAndSettingsDisabled: DisabledNavItem = {
  label: "Skills",
  icon: Sparkles,
  reason: "Coming in a later phase",
};

function SidebarLink({ item, collapsed }: { item: NavItem; collapsed: boolean }) {
  const link = (
    <NavLink
      to={item.to}
      className={({ isActive }) =>
        cn(
          "flex items-center gap-2.5 rounded-md px-2.5 py-1.5 text-sm font-medium transition-colors",
          isActive
            ? "bg-surface text-foreground"
            : "text-muted-foreground hover:bg-surface-hover hover:text-foreground",
          collapsed && "justify-center px-0",
        )
      }
    >
      <item.icon className="h-4 w-4 shrink-0" />
      {!collapsed && <span className="truncate">{item.label}</span>}
    </NavLink>
  );

  if (!collapsed) return link;

  return (
    <Tooltip delayDuration={200}>
      <TooltipTrigger asChild>{link}</TooltipTrigger>
      <TooltipContent side="right">{item.label}</TooltipContent>
    </Tooltip>
  );
}

function SidebarDisabledItem({ item, collapsed }: { item: DisabledNavItem; collapsed: boolean }) {
  return (
    <Tooltip delayDuration={200}>
      <TooltipTrigger asChild>
        <div
          className={cn(
            "flex cursor-not-allowed items-center gap-2.5 rounded-md px-2.5 py-1.5 text-sm font-medium text-subtle-foreground/60",
            collapsed && "justify-center px-0",
          )}
        >
          <item.icon className="h-4 w-4 shrink-0" />
          {!collapsed && <span className="truncate">{item.label}</span>}
        </div>
      </TooltipTrigger>
      <TooltipContent side="right">{item.reason}</TooltipContent>
    </Tooltip>
  );
}

function SidebarSectionLabel({ children, collapsed }: { children: string; collapsed: boolean }) {
  if (collapsed) return null;
  return (
    <div className="px-2.5 pb-1 pt-3 text-[11px] font-semibold uppercase tracking-wider text-subtle-foreground">
      {children}
    </div>
  );
}

export function Sidebar() {
  const collapsed = useUIStore((s) => s.sidebarCollapsed);
  const toggleSidebar = useUIStore((s) => s.toggleSidebar);
  const activeProjectId = useProjectStore((s) => s.activeProjectId);

  const projectItems: NavItem[] = activeProjectId
    ? [
        { label: "Files", to: `/projects/${activeProjectId}/files`, icon: Files },
        { label: "Changes", to: `/projects/${activeProjectId}/changes`, icon: GitCompare },
        { label: "Terminal", to: `/projects/${activeProjectId}/terminal`, icon: SquareTerminal },
        { label: "Testing", to: `/projects/${activeProjectId}/testing`, icon: FlaskConical },
      ]
    : [];

  const projectItemsDisabled: DisabledNavItem[] = activeProjectId
    ? []
    : [
        { label: "Files", icon: Files, reason: "Open a project to enable" },
        { label: "Changes", icon: GitCompare, reason: "Open a project to enable" },
        { label: "Terminal", icon: SquareTerminal, reason: "Open a project to enable" },
        { label: "Testing", icon: FlaskConical, reason: "Open a project to enable" },
      ];

  return (
    <aside
      className={cn(
        "flex h-full flex-col border-r border-border bg-background-elevated transition-[width] duration-150",
        collapsed ? "w-14" : "w-56",
      )}
    >
      <div
        className={cn(
          "flex h-12 items-center border-b border-border px-3",
          collapsed ? "justify-center" : "justify-between",
        )}
      >
        {!collapsed && (
          <div className="flex items-center gap-2 text-sm font-semibold text-foreground">
            <LayoutGrid className="h-4 w-4 text-primary" />
            Forge
          </div>
        )}
        <button
          type="button"
          onClick={toggleSidebar}
          className="flex h-6 w-6 items-center justify-center rounded text-subtle-foreground hover:bg-surface-hover hover:text-foreground"
          aria-label={collapsed ? "Expand sidebar" : "Collapse sidebar"}
        >
          {collapsed ? <ChevronsRight className="h-4 w-4" /> : <ChevronsLeft className="h-4 w-4" />}
        </button>
      </div>

      <nav className="flex flex-1 flex-col gap-0.5 overflow-y-auto px-2 py-2">
        <SidebarSectionLabel collapsed={collapsed}>Workspace</SidebarSectionLabel>
        {workspaceItems.map((item) => (
          <SidebarLink key={item.to} item={item} collapsed={collapsed} />
        ))}

        <div className="my-2 h-px bg-border" />

        <SidebarSectionLabel collapsed={collapsed}>Project</SidebarSectionLabel>
        {projectItems.map((item) => (
          <SidebarLink key={item.to} item={item} collapsed={collapsed} />
        ))}
        {projectItemsDisabled.map((item) => (
          <SidebarDisabledItem key={item.label} item={item} collapsed={collapsed} />
        ))}
        <SidebarDisabledItem
          item={{ label: "Project Brain", icon: BrainCircuit, reason: "Coming in a later phase" }}
          collapsed={collapsed}
        />

        <div className="my-2 h-px bg-border" />

        <SidebarDisabledItem item={skillsAndSettingsDisabled} collapsed={collapsed} />
        <SidebarLink item={{ label: "Settings", to: "/settings", icon: SettingsIcon }} collapsed={collapsed} />
      </nav>
    </aside>
  );
}
