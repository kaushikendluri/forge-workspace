import { Files } from "lucide-react";
import { EmptyState } from "@/components/empty-states/EmptyState";

export interface FilesTabProps {
  projectId: string;
}

/**
 * Real component for the Files tab of a project workspace. Unreachable via
 * navigation in M1 (ProjectWorkspaceLayout always shows "project not found"
 * since there's no backend to open a project), but built now so file-tree
 * wiring in a later milestone only has to fill this in, not design it.
 */
export function FilesTab({ projectId: _projectId }: FilesTabProps) {
  return (
    <EmptyState
      icon={Files}
      title="No files to show"
      description="File browsing connects once the desktop app backend is available."
    />
  );
}
