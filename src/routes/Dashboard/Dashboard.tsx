import { useEffect } from "react";
import { useNavigate } from "react-router-dom";
import { FolderGit2 } from "lucide-react";
import { EmptyState } from "@/components/empty-states/EmptyState";
import { useProjectStore } from "@/stores/useProjectStore";
import type { ProjectDto } from "@/types/db";

export function Dashboard() {
  const navigate = useNavigate();
  const projects = useProjectStore((s) => s.projects);
  const isLoading = useProjectStore((s) => s.isLoading);
  const error = useProjectStore((s) => s.error);
  const loadProjects = useProjectStore((s) => s.loadProjects);
  const openProjectDialog = useProjectStore((s) => s.openProjectDialog);
  const createProjectDialog = useProjectStore((s) => s.createProjectDialog);
  const setActiveProjectId = useProjectStore((s) => s.setActiveProjectId);

  useEffect(() => {
    void loadProjects();
  }, [loadProjects]);

  const handleOpen = async () => {
    const project = await openProjectDialog();
    if (project) navigate(`/projects/${project.id}`);
  };

  const handleCreate = async () => {
    const project = await createProjectDialog();
    if (project) navigate(`/projects/${project.id}`);
  };

  const handleSelect = (project: ProjectDto) => {
    setActiveProjectId(project.id);
    navigate(`/projects/${project.id}`);
  };

  if (projects.length > 0) {
    return (
      <div className="flex flex-col gap-3">
        <h1 className="text-lg font-semibold text-foreground">Dashboard</h1>
        {error && (
          <div className="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive">
            {error}
          </div>
        )}
        <ul className="flex flex-col gap-2">
          {projects.map((project) => (
            <li key={project.id}>
              <button
                type="button"
                onClick={() => handleSelect(project)}
                className="flex w-full flex-col gap-1 rounded-md border border-border bg-surface p-3 text-left text-sm hover:bg-surface-hover"
              >
                <span className="font-medium text-foreground">{project.name}</span>
                <span className="truncate text-xs text-muted-foreground">{project.rootPath}</span>
              </button>
            </li>
          ))}
        </ul>
      </div>
    );
  }

  return (
    <div className="flex flex-1 flex-col gap-4">
      <h1 className="text-lg font-semibold text-foreground">Dashboard</h1>
      {error && (
        <div className="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive">
          {error}
        </div>
      )}
      <EmptyState
        icon={FolderGit2}
        title="No projects yet"
        description="Open an existing repository or create a new project to get started."
        actions={[
          {
            label: isLoading ? "Opening…" : "Open an existing repository",
            onClick: handleOpen,
            disabled: isLoading,
          },
          {
            label: isLoading ? "Creating…" : "Create a new project",
            onClick: handleCreate,
            disabled: isLoading,
            variant: "secondary",
          },
        ]}
      />
    </div>
  );
}
