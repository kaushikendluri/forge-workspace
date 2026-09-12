import { useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import { FolderGit2, GitBranch } from "lucide-react";
import { EmptyState } from "@/components/empty-states/EmptyState";
import { useProjectStore } from "@/stores/useProjectStore";
import { gitCurrentBranch } from "@/lib/tauri";
import type { ProjectDto } from "@/types/db";

/** Fetches each listed project's real current branch via `git_current_branch`. */
function useLiveBranches(projects: ProjectDto[]): Record<string, string | null> {
  const [branches, setBranches] = useState<Record<string, string | null>>({});

  useEffect(() => {
    let cancelled = false;
    projects.forEach((project) => {
      gitCurrentBranch(project.rootPath)
        .then((branch) => {
          if (!cancelled) setBranches((prev) => ({ ...prev, [project.id]: branch }));
        })
        .catch(() => {
          if (!cancelled) setBranches((prev) => ({ ...prev, [project.id]: null }));
        });
    });
    return () => {
      cancelled = true;
    };
  }, [projects]);

  return branches;
}

export function Projects() {
  const navigate = useNavigate();
  const projects = useProjectStore((s) => s.projects);
  const isLoading = useProjectStore((s) => s.isLoading);
  const error = useProjectStore((s) => s.error);
  const loadProjects = useProjectStore((s) => s.loadProjects);
  const openProjectDialog = useProjectStore((s) => s.openProjectDialog);
  const createProjectDialog = useProjectStore((s) => s.createProjectDialog);
  const setActiveProjectId = useProjectStore((s) => s.setActiveProjectId);
  const branches = useLiveBranches(projects);

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

  return (
    <div className="flex flex-1 flex-col gap-4">
      <div className="flex items-center justify-between">
        <h1 className="text-lg font-semibold text-foreground">Projects</h1>
        {projects.length > 0 && (
          <div className="flex items-center gap-2">
            <button
              type="button"
              onClick={handleCreate}
              disabled={isLoading}
              className="inline-flex h-8 items-center justify-center rounded-md border border-border bg-transparent px-3 text-xs font-medium text-foreground hover:bg-surface-hover disabled:cursor-not-allowed disabled:opacity-50"
            >
              Create a new project
            </button>
            <button
              type="button"
              onClick={handleOpen}
              disabled={isLoading}
              className="inline-flex h-8 items-center justify-center rounded-md bg-primary px-3 text-xs font-medium text-primary-foreground hover:bg-primary/90 disabled:cursor-not-allowed disabled:opacity-50"
            >
              Open an existing repository
            </button>
          </div>
        )}
      </div>

      {error && (
        <div className="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive">
          {error}
        </div>
      )}

      {projects.length > 0 ? (
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
                <span className="flex items-center gap-3 text-xs text-subtle-foreground">
                  <span className="flex items-center gap-1">
                    <GitBranch className="h-3 w-3" />
                    {branches[project.id] ?? project.defaultBranch}
                  </span>
                  {project.lastOpenedAt && (
                    <span>Last opened {new Date(project.lastOpenedAt).toLocaleString()}</span>
                  )}
                </span>
              </button>
            </li>
          ))}
        </ul>
      ) : (
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
      )}
    </div>
  );
}
