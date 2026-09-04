import { FolderGit2 } from "lucide-react";
import { EmptyState } from "@/components/empty-states/EmptyState";
import { useProjectStore } from "@/stores/useProjectStore";

const BACKEND_DISABLED_REASON = "Available once the desktop app backend is connected (Phase 1 M2)";

export function Projects() {
  const projects = useProjectStore((s) => s.projects);

  if (projects.length > 0) {
    return (
      <div className="flex flex-col gap-3">
        <h1 className="text-lg font-semibold text-foreground">Projects</h1>
        <ul className="flex flex-col gap-2">
          {projects.map((project) => (
            <li key={project.id} className="rounded-md border border-border bg-surface p-3 text-sm">
              {project.name}
            </li>
          ))}
        </ul>
      </div>
    );
  }

  return (
    <div className="flex flex-1 flex-col gap-4">
      <h1 className="text-lg font-semibold text-foreground">Projects</h1>
      <EmptyState
        icon={FolderGit2}
        title="No projects yet"
        description="Open an existing repository or create a new project to get started."
        actions={[
          {
            label: "Open an existing repository",
            disabled: true,
            disabledReason: BACKEND_DISABLED_REASON,
          },
          {
            label: "Create a new project",
            variant: "secondary",
            disabled: true,
            disabledReason: BACKEND_DISABLED_REASON,
          },
        ]}
      />
    </div>
  );
}
