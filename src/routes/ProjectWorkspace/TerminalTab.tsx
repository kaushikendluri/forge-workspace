import { TerminalView } from "@/components/terminal/TerminalView";

export interface TerminalTabProps {
  projectId: string;
}

/**
 * Real component for the Terminal tab. Mounts a real xterm.js instance via
 * TerminalView, idle until PTY wiring lands (see `terminal_commands.rs`).
 * Unreachable via navigation in M1 for the same reason as FilesTab.
 */
export function TerminalTab({ projectId }: TerminalTabProps) {
  return (
    <div className="flex flex-1 flex-col gap-4">
      <h2 className="text-sm font-medium text-foreground">Terminal</h2>
      <TerminalView projectId={projectId} className="flex-1" />
    </div>
  );
}
