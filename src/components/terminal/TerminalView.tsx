import { useEffect, useRef } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import { cn } from "@/lib/utils";

export interface TerminalViewProps {
  projectId?: string;
  className?: string;
}

/**
 * Mounts a real xterm.js instance. There is no PTY backend yet (that's a
 * later milestone's `terminal_commands.rs` + portable-pty work), so this
 * stays idle and simply prints a status line — no simulated shell output.
 */
export function TerminalView({ className }: TerminalViewProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const terminalRef = useRef<Terminal | null>(null);

  useEffect(() => {
    if (!containerRef.current) return;

    const term = new Terminal({
      convertEol: true,
      cursorBlink: false,
      disableStdin: true,
      fontFamily: '"JetBrains Mono", "SF Mono", "Cascadia Code", monospace',
      fontSize: 13,
      theme: {
        background: "#00000000",
        foreground: "#c9d1d9",
      },
    });
    const fitAddon = new FitAddon();
    term.loadAddon(fitAddon);
    term.open(containerRef.current);
    fitAddon.fit();
    term.write("Terminal will connect once a project is open.\r\n");

    terminalRef.current = term;

    const resizeObserver = new ResizeObserver(() => {
      try {
        fitAddon.fit();
      } catch {
        // Container may be zero-size mid-transition; ignore and wait for the next resize.
      }
    });
    resizeObserver.observe(containerRef.current);

    return () => {
      resizeObserver.disconnect();
      term.dispose();
      terminalRef.current = null;
    };
  }, []);

  return (
    <div
      className={cn(
        "h-full min-h-[200px] w-full overflow-hidden rounded-lg border border-border bg-background-elevated p-2",
        className,
      )}
      ref={containerRef}
    />
  );
}
