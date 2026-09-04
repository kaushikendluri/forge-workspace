import { NavLink, Outlet, useParams } from "react-router-dom";
import { FolderX } from "lucide-react";
import { cn } from "@/lib/utils";
import { EmptyState } from "@/components/empty-states/EmptyState";
import { useProjectStore } from "@/stores/useProjectStore";

const tabs = [
  { label: "Files", to: "files" },
  { label: "Changes", to: "changes" },
  { label: "Terminal", to: "terminal" },
];

/**
 * Layout for `/projects/:projectId`. There is no backend in M1, so
 * `useProjectStore`'s `projects` list is always empty and this always shows
 * the "not found" state — the Files/Changes/Terminal tabs (and their nested
 * routes) are real but unreachable until a project can actually be opened.
 */
export function ProjectWorkspaceLayout() {
  const { projectId } = useParams<{ projectId: string }>();
  const project = useProjectStore((s) => s.projects.find((p) => p.id === projectId));

  if (!project) {
    return (
      <div className="flex flex-1 flex-col gap-4">
        <EmptyState
          icon={FolderX}
          title="Project not found"
          description="No backend connected yet — projects can't be opened until Phase 1 M2 lands."
        />
      </div>
    );
  }

  return (
    <div className="flex flex-1 flex-col gap-4">
      <h1 className="text-lg font-semibold text-foreground">{project.name}</h1>
      <nav className="flex items-center gap-1 border-b border-border">
        {tabs.map((tab) => (
          <NavLink
            key={tab.to}
            to={tab.to}
            className={({ isActive }) =>
              cn(
                "border-b-2 border-transparent px-3 py-2 text-sm font-medium text-muted-foreground transition-colors hover:text-foreground",
                isActive && "border-primary text-foreground",
              )
            }
          >
            {tab.label}
          </NavLink>
        ))}
      </nav>
      <Outlet context={{ projectId: project.id }} />
    </div>
  );
}
