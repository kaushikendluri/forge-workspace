import { Bot } from "lucide-react";
import { EmptyState } from "@/components/empty-states/EmptyState";

export function Agents() {
  return (
    <div className="flex flex-1 flex-col gap-4">
      <h1 className="text-lg font-semibold text-foreground">Agents</h1>
      <EmptyState icon={Bot} title="No agents yet" description="Open a project to get started." />
    </div>
  );
}
