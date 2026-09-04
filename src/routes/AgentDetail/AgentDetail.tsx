import { useParams } from "react-router-dom";
import { Bot } from "lucide-react";
import { EmptyState } from "@/components/empty-states/EmptyState";

/**
 * Stub for `/projects/:projectId/agents/:agentId`. No backend exists to run
 * or record agents yet, so this always renders a "not found" style empty
 * state rather than fabricating agent detail data.
 */
export function AgentDetail() {
  const { agentId } = useParams<{ projectId: string; agentId: string }>();

  return (
    <div className="flex flex-1 flex-col gap-4">
      <EmptyState
        icon={Bot}
        title="Agent not found"
        description={
          agentId
            ? `No backend connected yet — agent "${agentId}" can't be loaded until a later phase.`
            : "No backend connected yet."
        }
      />
    </div>
  );
}
