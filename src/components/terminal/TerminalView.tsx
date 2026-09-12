import { useEffect, useRef, useState } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import { cn } from "@/lib/utils";
import { terminalKill, terminalResize, terminalSpawn, terminalWrite } from "@/lib/tauri";
import { onForgeEvent } from "@/lib/events";
import { useTerminalStore } from "@/stores/useTerminalStore";

export interface TerminalViewProps {
  /** Working directory for the shell this view spawns — normally the active project's root path. */
  cwd: string;
  projectId: string;
  /** Kept mounted but visually hidden (rather than unmounted) so switching
   * tabs doesn't kill the underlying shell process. */
  hidden?: boolean;
  className?: string;
  onExit?: (exitCode: number | null) => void;
}

/** `terminal:output` chunks arrive base64-encoded (PTY output isn't
 * guaranteed valid UTF-8 on its own) — decode back to raw bytes for xterm,
 * which handles multi-byte sequences split across chunks correctly itself. */
function base64ToBytes(base64: string): Uint8Array {
  const binary = atob(base64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) {
    bytes[i] = binary.charCodeAt(i);
  }
  return bytes;
}

/**
 * Mounts a real xterm.js instance backed by a real PTY session: spawns a
 * shell via `terminal_spawn` on mount, forwards keystrokes to
 * `terminal_write`, writes incoming `terminal:output` events into the
 * terminal, keeps the pty's size in sync via `terminal_resize`, and kills
 * the session (`terminal_kill`) on unmount. No simulated output anywhere —
 * a spawn/write/resize failure is shown inline rather than swallowed.
 */
export function TerminalView({ cwd, projectId, hidden, className, onExit }: TerminalViewProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const terminalIdRef = useRef<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const addSession = useTerminalStore((s) => s.addSession);
  const updateSession = useTerminalStore((s) => s.updateSession);
  const removeSession = useTerminalStore((s) => s.removeSession);

  useEffect(() => {
    if (!containerRef.current) return;

    const term = new Terminal({
      convertEol: true,
      cursorBlink: true,
      fontFamily: '"JetBrains Mono", "SF Mono", "Cascadia Code", monospace',
      fontSize: 13,
      theme: { background: "#00000000", foreground: "#c9d1d9" },
    });
    const fitAddon = new FitAddon();
    term.loadAddon(fitAddon);
    term.open(containerRef.current);
    fitAddon.fit();
    term.write("Connecting to a real shell…\r\n");

    let cancelled = false;
    let unlistenOutput: (() => void) | undefined;
    let unlistenExit: (() => void) | undefined;
    let dataDisposable: { dispose: () => void } | undefined;

    void (async () => {
      let id: string;
      try {
        id = await terminalSpawn(cwd);
      } catch (err) {
        if (!cancelled) setError(err instanceof Error ? err.message : String(err));
        return;
      }
      if (cancelled) {
        void terminalKill(id).catch(() => {});
        return;
      }

      terminalIdRef.current = id;
      addSession({ id, projectId, status: "connected", exitCode: null });
      term.clear();

      unlistenOutput = await onForgeEvent("terminal:output", (payload) => {
        if (payload.terminalId !== id) return;
        term.write(base64ToBytes(payload.chunk));
      });
      unlistenExit = await onForgeEvent("terminal:exit", (payload) => {
        if (payload.terminalId !== id) return;
        updateSession(id, { status: "exited", exitCode: payload.exitCode });
        term.write(
          `\r\n[process exited${payload.exitCode !== null ? ` with code ${payload.exitCode}` : ""}]\r\n`,
        );
        onExit?.(payload.exitCode);
      });

      dataDisposable = term.onData((data) => {
        void terminalWrite(id, data).catch((err) => {
          setError(err instanceof Error ? err.message : String(err));
        });
      });

      const size = fitAddon.proposeDimensions();
      if (size) void terminalResize(id, size.cols, size.rows).catch(() => {});
    })();

    const resizeObserver = new ResizeObserver(() => {
      try {
        fitAddon.fit();
        const size = fitAddon.proposeDimensions();
        const id = terminalIdRef.current;
        if (id && size) void terminalResize(id, size.cols, size.rows).catch(() => {});
      } catch {
        // Container may be zero-size mid-transition; ignore and wait for the next resize.
      }
    });
    resizeObserver.observe(containerRef.current);

    return () => {
      cancelled = true;
      resizeObserver.disconnect();
      dataDisposable?.dispose();
      unlistenOutput?.();
      unlistenExit?.();
      const id = terminalIdRef.current;
      if (id) {
        void terminalKill(id).catch(() => {});
        removeSession(id);
      }
      term.dispose();
    };
    // Intentionally re-spawns only if `cwd`/`projectId` change, not on every
    // render (addSession/updateSession/removeSession are stable zustand
    // action references, and onExit is expected to be stable per caller).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [cwd, projectId]);

  return (
    <div className={cn("flex h-full min-h-[200px] w-full flex-col gap-2", className)} hidden={hidden}>
      {error && (
        <div className="rounded-md border border-destructive/40 bg-destructive/10 px-2 py-1.5 text-xs text-destructive">
          {error}
        </div>
      )}
      <div
        ref={containerRef}
        className="min-h-0 flex-1 overflow-hidden rounded-lg border border-border bg-background-elevated p-2"
      />
    </div>
  );
}
