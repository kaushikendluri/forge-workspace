import { ListChecks } from "lucide-react";
import { EmptyState } from "@/components/empty-states/EmptyState";

export function Tasks() {
  return (
    <div className="flex flex-1 flex-col gap-4">
      <h1 className="text-lg font-semibold text-foreground">Tasks</h1>
      <EmptyState icon={ListChecks} title="No tasks yet" description="Open a project to get started." />
    </div>
  );
}
