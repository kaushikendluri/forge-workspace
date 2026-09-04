import { Activity as ActivityIcon } from "lucide-react";
import { EmptyState } from "@/components/empty-states/EmptyState";

export function Activity() {
  return (
    <div className="flex flex-1 flex-col gap-4">
      <h1 className="text-lg font-semibold text-foreground">Activity</h1>
      <EmptyState
        icon={ActivityIcon}
        title="No activity yet"
        description="Open a project to get started."
      />
    </div>
  );
}
