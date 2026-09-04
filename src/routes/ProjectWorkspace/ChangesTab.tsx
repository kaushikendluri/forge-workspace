import { MonacoDiffViewer } from "@/components/diff/MonacoDiffViewer";

export interface ChangesTabProps {
  projectId: string;
}

/**
 * Real component for the Changes tab. No diff source exists until a later
 * milestone wires this to `git_commands.rs`, so it renders the diff viewer
 * shell with no original/modified content — MonacoDiffViewer itself shows
 * the "No diff to display" empty state in that case. Unreachable via
 * navigation in M1 for the same reason as FilesTab.
 */
export function ChangesTab({ projectId: _projectId }: ChangesTabProps) {
  return (
    <div className="flex flex-1 flex-col gap-4">
      <h2 className="text-sm font-medium text-foreground">Changes</h2>
      <MonacoDiffViewer />
    </div>
  );
}
