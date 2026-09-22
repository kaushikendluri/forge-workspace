import { useEffect, useMemo, useState } from "react";
import { Link, useParams } from "react-router-dom";
import { Bot, FileDiff, Square } from "lucide-react";
import { EmptyState } from "@/components/empty-states/EmptyState";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { MonacoDiffViewer } from "@/components/diff/MonacoDiffViewer";
import { useAgentStore } from "@/stores/useAgentStore";
import { useSettingsStore } from "@/stores/useSettingsStore";
import {
  abortAgentRunMerge,
  acceptVisualSnapshot,
  createVisualRegressionFollowUpTask,
  flagVisualSnapshot,
  getAgentRun,
  getMergeReadiness,
  getReview,
  getRunDiff,
  getVisualSnapshotImage,
  listActivityEvents,
  listAgentRuns,
  listAgents,
  listToolCalls,
  listVisualSnapshots,
  mergeAgentRun,
  requestReview,
  resolveAgentRunMergeConflictsWithAgent,
  startAgentRun,
  stopAgentRun,
} from "@/lib/tauri";
import { onForgeEvent } from "@/lib/events";
import { toastError } from "@/stores/useToastStore";
import { cn } from "@/lib/utils";
import type {
  Agent,
  AgentRunFileDiffDto,
  AgentRunStatus,
  MergeReadinessDto,
  MergeResultDto,
  ReadinessCheck,
  ReviewCategory,
  ReviewDto,
  ReviewSeverity,
  ToolCallStatus,
  VisualSnapshotDto,
} from "@/types/db";

function errorMessage(err: unknown): string {
  if (typeof err === "string") return err;
  if (err instanceof Error) return err.message;
  return String(err);
}

const RUN_STATUS_BADGE: Record<AgentRunStatus, { label: string; variant: "default" | "success" | "destructive" | "warning" | "secondary" }> = {
  queued: { label: "Queued", variant: "secondary" },
  running: { label: "Running", variant: "default" },
  completed: { label: "Completed", variant: "success" },
  failed: { label: "Failed", variant: "destructive" },
  stopped: { label: "Stopped", variant: "warning" },
};

const TOOL_CALL_BADGE: Record<ToolCallStatus, { label: string; variant: "default" | "success" | "destructive" }> = {
  running: { label: "running", variant: "default" },
  success: { label: "success", variant: "success" },
  error: { label: "error", variant: "destructive" },
};

/** M14: findings are grouped for display in this fixed category order. */
const REVIEW_CATEGORY_ORDER: ReviewCategory[] = [
  "correctness",
  "security",
  "performance",
  "maintainability",
  "tests",
  "architecture",
  "style",
];

const REVIEW_SEVERITY_BADGE: Record<ReviewSeverity, { label: string; variant: "secondary" | "default" | "warning" | "destructive" }> = {
  low: { label: "low", variant: "secondary" },
  medium: { label: "medium", variant: "default" },
  high: { label: "high", variant: "warning" },
  critical: { label: "critical", variant: "destructive" },
};

const REVIEW_STATUS_BADGE: Record<ReviewDto["status"], { label: string; variant: "default" | "success" | "destructive" }> = {
  pending: { label: "Reviewing…", variant: "default" },
  passed: { label: "Passed", variant: "success" },
  failed: { label: "Failed", variant: "destructive" },
};

/**
 * M14: this run's real reviewer verdict — score, pass/fail status, and
 * findings grouped by category with a severity badge — plus a "Request
 * review" action when none has been requested yet. Only meaningful for a
 * `completed` run (only `completed` runs can be reviewed — see
 * `agent::reviewer::run_review`'s validation); shows an honest "no review
 * yet" state rather than fabricating one, and a live "in progress" state
 * while a `pending` review (manually requested here, or auto-triggered by
 * the scheduler once a mission task completes) is in flight.
 */
function ReviewPanel({ agentRunId }: { agentRunId: string }) {
  const [review, setReview] = useState<ReviewDto | null | undefined>(undefined);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [isRequesting, setIsRequesting] = useState(false);
  const [requestError, setRequestError] = useState<string | null>(null);

  const refetch = () => {
    getReview(agentRunId)
      .then((result) => setReview(result))
      .catch((err) => setLoadError(errorMessage(err)));
  };

  useEffect(() => {
    let cancelled = false;
    setReview(undefined);
    setLoadError(null);
    getReview(agentRunId)
      .then((result) => {
        if (!cancelled) setReview(result);
      })
      .catch((err) => {
        if (!cancelled) setLoadError(errorMessage(err));
      });
    return () => {
      cancelled = true;
    };
  }, [agentRunId]);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    void (async () => {
      unlisten = await onForgeEvent("review:updated", (payload) => {
        if (!cancelled && payload.agentRunId === agentRunId) refetch();
      });
    })();
    return () => {
      cancelled = true;
      unlisten?.();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [agentRunId]);

  const handleRequest = async () => {
    setIsRequesting(true);
    setRequestError(null);
    try {
      const result = await requestReview(agentRunId);
      setReview(result);
    } catch (err) {
      setRequestError(errorMessage(err));
    } finally {
      setIsRequesting(false);
    }
  };

  if (loadError) {
    return (
      <div className="rounded-md border border-destructive/40 bg-destructive/10 px-2 py-1.5 text-xs text-destructive">
        Couldn't load this run's review: {loadError}
      </div>
    );
  }
  if (review === undefined) {
    return <p className="text-xs text-muted-foreground">Loading review…</p>;
  }
  if (review === null) {
    return (
      <div className="flex flex-col gap-2">
        <p className="text-xs text-muted-foreground">No review requested yet for this run.</p>
        <div>
          <Button size="sm" onClick={() => void handleRequest()} disabled={isRequesting}>
            {isRequesting ? "Requesting review…" : "Request review"}
          </Button>
        </div>
        {requestError && (
          <div className="rounded-md border border-destructive/40 bg-destructive/10 px-2 py-1.5 text-xs text-destructive">
            {requestError}
          </div>
        )}
      </div>
    );
  }

  if (review.status === "pending") {
    return <p className="text-xs text-muted-foreground">A real reviewer agent is investigating this change…</p>;
  }

  const statusBadge = REVIEW_STATUS_BADGE[review.status];
  const findingsByCategory = new Map<ReviewCategory, typeof review.findings>();
  for (const finding of review.findings) {
    const list = findingsByCategory.get(finding.category) ?? [];
    list.push(finding);
    findingsByCategory.set(finding.category, list);
  }

  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center gap-2">
        <span className="text-sm font-medium text-foreground">Score: {review.score}/100</span>
        <Badge variant={statusBadge.variant}>{statusBadge.label}</Badge>
        <span className="text-[11px] text-subtle-foreground">{new Date(review.createdAt).toLocaleString()}</span>
      </div>
      {review.findings.length === 0 && <p className="text-xs text-muted-foreground">No findings — clean review.</p>}
      {review.findings.length > 0 && (
        <div className="flex flex-col gap-2">
          {REVIEW_CATEGORY_ORDER.filter((category) => findingsByCategory.has(category)).map((category) => (
            <div key={category} className="flex flex-col gap-1">
              <h3 className="text-[11px] font-semibold uppercase tracking-wide text-subtle-foreground">{category}</h3>
              {findingsByCategory.get(category)!.map((finding, i) => {
                const severityBadge = REVIEW_SEVERITY_BADGE[finding.severity];
                return (
                  <div key={i} className="rounded border border-border/60 bg-surface px-2 py-1.5 text-xs">
                    <div className="flex items-center justify-between gap-2">
                      <Badge variant={severityBadge.variant}>{severityBadge.label}</Badge>
                      {finding.file && (
                        <span className="text-[10px] text-subtle-foreground">
                          {finding.file}
                          {finding.line !== null ? `:${finding.line}` : ""}
                        </span>
                      )}
                    </div>
                    <p className="mt-1 text-muted-foreground">{finding.summary}</p>
                  </div>
                );
              })}
            </div>
          ))}
        </div>
      )}
      <div>
        <Button size="sm" variant="secondary" onClick={() => void handleRequest()} disabled={isRequesting}>
          {isRequesting ? "Requesting review…" : "Request another review"}
        </Button>
      </div>
      {requestError && (
        <div className="rounded-md border border-destructive/40 bg-destructive/10 px-2 py-1.5 text-xs text-destructive">
          {requestError}
        </div>
      )}
    </div>
  );
}

const READINESS_BADGE: Record<ReadinessCheck, { label: string; variant: "secondary" | "success" | "destructive" }> = {
  not_run: { label: "Not run", variant: "secondary" },
  passed: { label: "Passed", variant: "success" },
  failed: { label: "Failed", variant: "destructive" },
};

function ReadinessRow({ label, check }: { label: string; check: ReadinessCheck }) {
  const badge = READINESS_BADGE[check];
  return (
    <div className="flex items-center justify-between gap-2 text-xs">
      <span className="text-muted-foreground">{label}</span>
      <Badge variant={badge.variant}>{badge.label}</Badge>
    </div>
  );
}

/**
 * M15: a real merge-readiness checklist (tests/build/review/conflicts, all
 * genuine tri-state signals — never a fabricated checkmark for something
 * that hasn't actually run) plus the real merge action, with an AI
 * conflict-resolver fallback and an honest "abort" path when a real merge
 * attempt conflicts. Only meaningful for a `completed` run, same as
 * `ReviewPanel`.
 */
function MergePanel({ agentRunId }: { agentRunId: string }) {
  const [readiness, setReadiness] = useState<MergeReadinessDto | undefined>(undefined);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [changedFileCount, setChangedFileCount] = useState<number | null>(null);
  const [mergeResult, setMergeResult] = useState<MergeResultDto | null>(null);
  const [isMerging, setIsMerging] = useState(false);
  const [mergeError, setMergeError] = useState<string | null>(null);
  const [isResolving, setIsResolving] = useState(false);
  const [resolveError, setResolveError] = useState<string | null>(null);
  const [isAborting, setIsAborting] = useState(false);

  const hasApiKey = useSettingsStore((s) => s.hasApiKey);

  const refetchReadiness = () => {
    getMergeReadiness(agentRunId)
      .then((result) => setReadiness(result))
      .catch((err) => setLoadError(errorMessage(err)));
  };

  useEffect(() => {
    let cancelled = false;
    setReadiness(undefined);
    setLoadError(null);
    setMergeResult(null);
    getMergeReadiness(agentRunId)
      .then((result) => {
        if (!cancelled) setReadiness(result);
      })
      .catch((err) => {
        if (!cancelled) setLoadError(errorMessage(err));
      });
    // File count for the "N files changed" stat — reuses the same diff data
    // `RunDiffPanel` shows, just counted here rather than rendered.
    getRunDiff(agentRunId)
      .then((files) => {
        if (!cancelled) setChangedFileCount(files.length);
      })
      .catch(() => {
        // Non-critical — the checklist itself is what matters here.
      });
    return () => {
      cancelled = true;
    };
  }, [agentRunId]);

  const handleMerge = async () => {
    setIsMerging(true);
    setMergeError(null);
    try {
      const result = await mergeAgentRun(agentRunId);
      setMergeResult(result);
      if (result.merged) refetchReadiness();
    } catch (err) {
      setMergeError(errorMessage(err));
    } finally {
      setIsMerging(false);
    }
  };

  const handleAbort = async () => {
    setIsAborting(true);
    setMergeError(null);
    try {
      await abortAgentRunMerge(agentRunId);
      setMergeResult(null);
      refetchReadiness();
    } catch (err) {
      setMergeError(errorMessage(err));
    } finally {
      setIsAborting(false);
    }
  };

  const handleResolve = async () => {
    setIsResolving(true);
    setResolveError(null);
    try {
      await resolveAgentRunMergeConflictsWithAgent(agentRunId);
      setMergeResult({ merged: true, conflicts: [] });
      refetchReadiness();
    } catch (err) {
      setResolveError(errorMessage(err));
    } finally {
      setIsResolving(false);
    }
  };

  if (loadError) {
    return (
      <div className="rounded-md border border-destructive/40 bg-destructive/10 px-2 py-1.5 text-xs text-destructive">
        Couldn't check merge readiness: {loadError}
      </div>
    );
  }
  if (readiness === undefined) {
    return <p className="text-xs text-muted-foreground">Checking merge readiness…</p>;
  }

  // A conflict can come from either the live readiness dry run (before any
  // merge attempt) or a real merge attempt's own result — whichever is more
  // recent (the merge result, once one exists) wins.
  const conflicts = mergeResult ? mergeResult.conflicts : readiness.noConflicts === "failed" ? readiness.conflictedFiles : [];
  const inConflict = conflicts.length > 0;
  const merged = mergeResult?.merged === true;

  return (
    <div className="flex flex-col gap-3">
      <div className="grid grid-cols-2 gap-x-6 gap-y-1 sm:grid-cols-4">
        <ReadinessRow label="Tests" check={readiness.tests} />
        <ReadinessRow label="Build" check={readiness.build} />
        <ReadinessRow label="Review" check={readiness.review} />
        <ReadinessRow label="No conflicts" check={readiness.noConflicts} />
      </div>
      {changedFileCount !== null && (
        <p className="text-[11px] text-subtle-foreground">
          {changedFileCount} file{changedFileCount === 1 ? "" : "s"} changed
        </p>
      )}

      {merged && (
        <div className="rounded-md border border-success/40 bg-success/10 px-2 py-1.5 text-xs text-success">
          Merged successfully into the base branch.
        </div>
      )}

      {!merged && inConflict && (
        <div className="flex flex-col gap-2 rounded-md border border-destructive/40 bg-destructive/10 px-2 py-1.5 text-xs">
          <p className="font-medium text-destructive">
            Merge conflicts in {conflicts.length} file{conflicts.length === 1 ? "" : "s"}:
          </p>
          <ul className="list-inside list-disc text-destructive">
            {conflicts.map((f) => (
              <li key={f.path} className="font-mono">
                {f.path} <span className="text-subtle-foreground">({f.statusCode})</span>
              </li>
            ))}
          </ul>
          <div className="flex flex-wrap gap-2">
            {hasApiKey && (
              <Button size="sm" onClick={() => void handleResolve()} disabled={isResolving || isAborting}>
                {isResolving ? "Asking agent to resolve…" : "Ask agent to resolve"}
              </Button>
            )}
            <Button size="sm" variant="destructive" onClick={() => void handleAbort()} disabled={isResolving || isAborting}>
              {isAborting ? "Aborting…" : "Abort merge"}
            </Button>
          </div>
          {resolveError && <p className="text-destructive">{resolveError}</p>}
        </div>
      )}

      {!merged && !inConflict && (
        <div>
          <Button size="sm" onClick={() => void handleMerge()} disabled={isMerging || !readiness.canMerge}>
            {isMerging ? "Merging…" : "Merge"}
          </Button>
          {!readiness.canMerge && <p className="mt-1 text-[11px] text-subtle-foreground">Resolve conflicts before merging.</p>}
        </div>
      )}

      {mergeError && (
        <div className="rounded-md border border-destructive/40 bg-destructive/10 px-2 py-1.5 text-xs text-destructive">{mergeError}</div>
      )}
    </div>
  );
}

/**
 * M16: real visual regression snapshots captured by the agent's
 * `browser_screenshot` tool, grouped by label — baseline vs. each
 * comparison, with Accept/Reject/Ask-to-fix actions. Shows an honest empty
 * state rather than fabricating a comparison when the agent never called
 * `browser_screenshot`. "Accept" promotes a comparison to the new baseline
 * (a plain DB update); "Reject" only flags it for attention — it never
 * pretends to auto-revert anything; "Ask to fix" files a real follow-up
 * task (into the run's mission if it has one, otherwise a standalone
 * project task).
 */
function VisualRegressionPanel({ agentRunId }: { agentRunId: string }) {
  const [snapshots, setSnapshots] = useState<VisualSnapshotDto[] | null>(null);
  const [images, setImages] = useState<Record<string, string>>({});
  const [loadError, setLoadError] = useState<string | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [followUpTitles, setFollowUpTitles] = useState<Record<string, string>>({});

  const refetch = () => {
    listVisualSnapshots(agentRunId)
      .then((result) => setSnapshots(result))
      .catch((err) => setLoadError(errorMessage(err)));
  };

  useEffect(() => {
    let cancelled = false;
    setSnapshots(null);
    setLoadError(null);
    listVisualSnapshots(agentRunId)
      .then((result) => {
        if (!cancelled) setSnapshots(result);
      })
      .catch((err) => {
        if (!cancelled) setLoadError(errorMessage(err));
      });
    return () => {
      cancelled = true;
    };
  }, [agentRunId]);

  // Lazily fetches each snapshot's real image bytes once its row is known —
  // a separate round trip per image (rather than inlining bytes into
  // `listVisualSnapshots`) since a run can accumulate many screenshots and
  // most views only need to render a handful at a time.
  useEffect(() => {
    if (!snapshots || snapshots.length === 0) return;
    let cancelled = false;
    void (async () => {
      for (const snapshot of snapshots) {
        if (snapshot.id in images) continue;
        try {
          const data = await getVisualSnapshotImage(snapshot.id);
          if (!cancelled) setImages((prev) => ({ ...prev, [snapshot.id]: data }));
        } catch {
          // Non-critical — that one image just won't render.
        }
      }
    })();
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [snapshots]);

  const handleAccept = async (snapshotId: string) => {
    setBusyId(snapshotId);
    setActionError(null);
    try {
      await acceptVisualSnapshot(snapshotId);
      refetch();
    } catch (err) {
      setActionError(errorMessage(err));
    } finally {
      setBusyId(null);
    }
  };

  const handleReject = async (snapshotId: string) => {
    setBusyId(snapshotId);
    setActionError(null);
    try {
      await flagVisualSnapshot(snapshotId);
      refetch();
    } catch (err) {
      setActionError(errorMessage(err));
    } finally {
      setBusyId(null);
    }
  };

  const handleAskToFix = async (snapshotId: string) => {
    setBusyId(snapshotId);
    setActionError(null);
    try {
      const task = await createVisualRegressionFollowUpTask(snapshotId);
      setFollowUpTitles((prev) => ({ ...prev, [snapshotId]: task.title }));
    } catch (err) {
      setActionError(errorMessage(err));
    } finally {
      setBusyId(null);
    }
  };

  if (loadError) {
    return (
      <div className="rounded-md border border-destructive/40 bg-destructive/10 px-2 py-1.5 text-xs text-destructive">
        Couldn't load visual snapshots: {loadError}
      </div>
    );
  }
  if (snapshots === null) {
    return <p className="text-xs text-muted-foreground">Loading visual snapshots…</p>;
  }
  if (snapshots.length === 0) {
    return (
      <p className="text-xs text-muted-foreground">
        No screenshots captured yet — the agent hasn't used the browser tools for this run.
      </p>
    );
  }

  const byLabel = new Map<string, VisualSnapshotDto[]>();
  for (const snapshot of snapshots) {
    const list = byLabel.get(snapshot.label) ?? [];
    list.push(snapshot);
    byLabel.set(snapshot.label, list);
  }

  return (
    <div className="flex flex-col gap-4">
      {Array.from(byLabel.entries()).map(([label, group]) => {
        const baseline = group.find((s) => s.kind === "baseline") ?? null;
        const comparisons = group.filter((s) => s.kind === "comparison");
        return (
          <div key={label} className="flex flex-col gap-2 rounded-md border border-border/60 p-2">
            <h3 className="text-xs font-semibold text-foreground">{label}</h3>
            <div className="flex flex-wrap gap-3">
              {baseline && (
                <div className="flex flex-col gap-1">
                  <Badge variant="secondary">Baseline</Badge>
                  {images[baseline.id] && (
                    <img
                      src={`data:image/png;base64,${images[baseline.id]}`}
                      alt={`${label} baseline`}
                      className="w-64 rounded border border-border"
                    />
                  )}
                </div>
              )}
              {comparisons.length === 0 && (
                <p className="self-center text-[11px] text-muted-foreground">No comparisons yet for this label.</p>
              )}
              {comparisons.map((comparison) => (
                <div key={comparison.id} className="flex flex-col gap-1">
                  <div className="flex items-center gap-1.5">
                    <Badge variant={comparison.flagged ? "warning" : "default"}>
                      {comparison.flagged ? "Flagged" : "Comparison"}
                    </Badge>
                    <span className="text-[10px] text-subtle-foreground">
                      {new Date(comparison.createdAt).toLocaleTimeString()}
                    </span>
                  </div>
                  {images[comparison.id] && (
                    <img
                      src={`data:image/png;base64,${images[comparison.id]}`}
                      alt={`${label} comparison`}
                      className="w-64 rounded border border-border"
                    />
                  )}
                  <div className="flex flex-wrap gap-1.5">
                    <Button
                      size="sm"
                      variant="secondary"
                      onClick={() => void handleAccept(comparison.id)}
                      disabled={busyId === comparison.id}
                    >
                      Accept
                    </Button>
                    <Button
                      size="sm"
                      variant="destructive"
                      onClick={() => void handleReject(comparison.id)}
                      disabled={busyId === comparison.id}
                    >
                      Reject
                    </Button>
                    <Button size="sm" onClick={() => void handleAskToFix(comparison.id)} disabled={busyId === comparison.id}>
                      Ask to fix
                    </Button>
                  </div>
                  {followUpTitles[comparison.id] && (
                    <p className="max-w-64 text-[11px] text-success">Follow-up task created: {followUpTitles[comparison.id]}</p>
                  )}
                </div>
              ))}
            </div>
          </div>
        );
      })}
      {actionError && (
        <div className="rounded-md border border-destructive/40 bg-destructive/10 px-2 py-1.5 text-xs text-destructive">
          {actionError}
        </div>
      )}
    </div>
  );
}

function formatActivitySummary(payloadJson: string): string {
  try {
    const parsed = JSON.parse(payloadJson) as Record<string, unknown>;
    // M11: a `send_message` tool call's own activity entry — surface it as a
    // message rather than raw tool-call JSON, reusing this same real
    // activity-stream entry rather than a separate UI.
    if (parsed.toolName === "send_message" && typeof parsed.output === "string") {
      return `Message: ${parsed.output}`;
    }
    // M13: the self-healing test-fix cycle's own activity entries — see
    // `agent::tool_loop::test_fix_event_payload` for the payload shapes.
    if (parsed.phase === "failed" && typeof parsed.attempt === "number") {
      return `Attempt ${parsed.attempt}: tests failed → agent is investigating`;
    }
    if (parsed.phase === "retested" && typeof parsed.attempt === "number") {
      return `Attempt ${parsed.attempt}: retest ${parsed.passed ? "passed" : "still failing"}`;
    }
    if (parsed.phase === "budget_exhausted" && typeof parsed.attempts === "number") {
      const plural = parsed.attempts === 1 ? "" : "s";
      return `Test-fix budget exhausted after ${parsed.attempts} attempt${plural} (cap ${parsed.cap}) — tests still failing; stopping for a human to look.`;
    }
    if (typeof parsed.text === "string" && parsed.text.trim().length > 0) return parsed.text;
    if (typeof parsed.summary === "string") return parsed.summary;
    if (typeof parsed.errorMessage === "string" && parsed.errorMessage) return parsed.errorMessage;
    if (typeof parsed.taskPrompt === "string") return `Task: ${parsed.taskPrompt}`;
    return payloadJson;
  } catch {
    return payloadJson;
  }
}

/** Diff panel: file list + `MonacoDiffViewer`, mirroring `ChangesTab.tsx`'s layout. */
function RunDiffPanel({ agentRunId }: { agentRunId: string }) {
  const [files, setFiles] = useState<AgentRunFileDiffDto[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [selectedPath, setSelectedPath] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setFiles(null);
    setError(null);
    setSelectedPath(null);
    getRunDiff(agentRunId)
      .then((result) => {
        if (cancelled) return;
        setFiles(result);
        setSelectedPath(result[0]?.path ?? null);
      })
      .catch((err) => {
        if (!cancelled) setError(errorMessage(err));
      });
    return () => {
      cancelled = true;
    };
  }, [agentRunId]);

  const selected = files?.find((f) => f.path === selectedPath) ?? null;

  if (error) {
    return (
      <div className="rounded-md border border-destructive/40 bg-destructive/10 px-2 py-1.5 text-xs text-destructive">
        {error}
      </div>
    );
  }
  if (!files) {
    return <p className="text-xs text-muted-foreground">Loading diff…</p>;
  }
  if (files.length === 0) {
    return <EmptyState icon={FileDiff} title="No changes" description="This run's workspace has no uncommitted changes." />;
  }

  return (
    <div className="flex flex-1 gap-4 overflow-hidden">
      <div className="flex w-56 shrink-0 flex-col gap-0.5 overflow-y-auto">
        {files.map((file) => (
          <button
            key={file.path}
            type="button"
            title={file.path}
            onClick={() => setSelectedPath(file.path)}
            className={cn(
              "truncate rounded px-2 py-1 text-left text-xs text-foreground hover:bg-surface-hover",
              selectedPath === file.path && "bg-surface-hover",
            )}
          >
            {file.path}
          </button>
        ))}
      </div>
      <MonacoDiffViewer
        original={selected?.original}
        modified={selected?.modified}
        path={selected?.path}
        className="flex-1"
      />
    </div>
  );
}

/**
 * Real Agent Detail page: shows the current run for `agentId` (its task
 * prompt, live status, activity feed, tool call cards, and a diff of what
 * changed), with a working Start (for a `queued` run) and Stop (for a
 * `running` one) button. All live data comes from `agent-run:*` events via
 * `useAgentStore` — nothing here is simulated.
 */
export function AgentDetail() {
  const { projectId, agentId } = useParams<{ projectId: string; agentId: string }>();

  const [agent, setAgent] = useState<Agent | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [currentRunId, setCurrentRunId] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [isStarting, setIsStarting] = useState(false);
  const [isStopping, setIsStopping] = useState(false);

  const hasApiKey = useSettingsStore((s) => s.hasApiKey);
  const checkApiKey = useSettingsStore((s) => s.checkApiKey);

  const loadRun = useAgentStore((s) => s.loadRun);
  const setRun = useAgentStore((s) => s.setRun);
  const appendActivity = useAgentStore((s) => s.appendActivity);
  const upsertToolCall = useAgentStore((s) => s.upsertToolCall);
  const appendMessageDelta = useAgentStore((s) => s.appendMessageDelta);
  const runState = useAgentStore((s) => (currentRunId ? s.runsById[currentRunId] : undefined));

  useEffect(() => {
    void checkApiKey();
  }, [checkApiKey]);

  // Initial load: the agent row, its most recent run, and that run's full
  // tool-call/activity history so far.
  useEffect(() => {
    if (!projectId || !agentId) return;
    let cancelled = false;
    setIsLoading(true);
    setLoadError(null);
    setCurrentRunId(null);

    (async () => {
      const agents = await listAgents(projectId);
      const found = agents.find((a) => a.id === agentId) ?? null;
      if (cancelled) return;
      setAgent(found);
      if (!found) return;

      const runs = await listAgentRuns(agentId);
      const latest = runs[0] ?? null;
      if (cancelled || !latest) return;

      const [toolCalls, activity] = await Promise.all([listToolCalls(latest.id), listActivityEvents(latest.id)]);
      if (cancelled) return;
      loadRun(latest.id, latest, activity, toolCalls);
      setCurrentRunId(latest.id);
    })()
      .catch((err) => {
        if (!cancelled) setLoadError(errorMessage(err));
      })
      .finally(() => {
        if (!cancelled) setIsLoading(false);
      });

    return () => {
      cancelled = true;
    };
  }, [projectId, agentId, loadRun]);

  // Live updates for the current run, once known.
  useEffect(() => {
    if (!currentRunId) return;
    let cancelled = false;
    const unlisten: Array<() => void> = [];

    void (async () => {
      unlisten.push(
        await onForgeEvent("agent-run:status-changed", (payload) => {
          if (cancelled || payload.agentRunId !== currentRunId) return;
          // The event payload only carries the new status; refetch the full
          // row (token counts, stop reason, timestamps) rather than
          // synthesizing a partial one. This runs in the background (not
          // from a user action), so a failure has no natural inline banner
          // to land in — surface it via toast instead of swallowing it.
          getAgentRun(currentRunId)
            .then((run) => {
              if (!cancelled) setRun(currentRunId, run);
            })
            .catch((err) => {
              if (!cancelled) toastError("Couldn't refresh run status", errorMessage(err));
            });
        }),
      );
      unlisten.push(
        await onForgeEvent("agent-run:activity", (payload) => {
          if (!cancelled && payload.agentRunId === currentRunId) appendActivity(currentRunId, payload.event);
        }),
      );
      unlisten.push(
        await onForgeEvent("agent-run:tool-call-updated", (payload) => {
          if (!cancelled && payload.agentRunId === currentRunId) upsertToolCall(currentRunId, payload.toolCall);
        }),
      );
      unlisten.push(
        await onForgeEvent("agent-run:message-delta", (payload) => {
          if (!cancelled && payload.agentRunId === currentRunId) appendMessageDelta(currentRunId, payload.text);
        }),
      );
    })();

    return () => {
      cancelled = true;
      unlisten.forEach((fn) => fn());
    };
  }, [currentRunId, setRun, appendActivity, upsertToolCall, appendMessageDelta]);

  const run = runState?.run ?? null;
  const activity = useMemo(() => runState?.activity ?? [], [runState]);
  const toolCalls = useMemo(
    () => (runState ? runState.toolCallOrder.map((id) => runState.toolCallsById[id]) : []),
    [runState],
  );
  const streamingText = runState?.streamingText ?? "";

  const handleStart = async () => {
    if (!run) return;
    setActionError(null);
    setIsStarting(true);
    try {
      await startAgentRun(run.id);
    } catch (err) {
      setActionError(errorMessage(err));
    } finally {
      setIsStarting(false);
    }
  };

  const handleStop = async () => {
    if (!run) return;
    setActionError(null);
    setIsStopping(true);
    try {
      await stopAgentRun(run.id);
    } catch (err) {
      setActionError(errorMessage(err));
    } finally {
      setIsStopping(false);
    }
  };

  if (isLoading) {
    return <p className="p-4 text-xs text-muted-foreground">Loading…</p>;
  }

  if (loadError || !agent) {
    return (
      <EmptyState
        icon={Bot}
        title="Agent not found"
        description={loadError ?? `No agent "${agentId}" found for this project.`}
      />
    );
  }

  if (!run) {
    return (
      <div className="flex flex-1 flex-col items-center justify-center gap-3">
        <EmptyState
          icon={Bot}
          title="No run yet"
          description={`"${agent.name}" has no runs. Go to Agents and click Start to create one.`}
        />
        <Button asChild variant="secondary" size="sm">
          <Link to="/agents">Go to Agents</Link>
        </Button>
      </div>
    );
  }

  const statusBadge = RUN_STATUS_BADGE[run.status];
  const isRunning = run.status === "running";
  const isQueued = run.status === "queued";
  const isTerminal = run.status === "completed" || run.status === "failed" || run.status === "stopped";

  return (
    <div className="flex flex-1 flex-col gap-4 overflow-hidden">
      <div className="flex items-start justify-between gap-3">
        <div className="flex flex-col gap-1">
          <div className="flex items-center gap-2">
            <h1 className="text-lg font-semibold text-foreground">{agent.name}</h1>
            <Badge variant={statusBadge.variant}>{statusBadge.label}</Badge>
          </div>
          <p className="max-w-2xl text-xs text-muted-foreground">{run.taskPrompt}</p>
          <p className="text-[11px] text-subtle-foreground">
            model {run.modelId} · iteration {run.iterationCount} · {run.totalInputTokens.toLocaleString()} in /{" "}
            {run.totalOutputTokens.toLocaleString()} out tokens
            {run.stopReason && ` · stop reason: ${run.stopReason}`}
          </p>
          {run.errorMessage && <p className="max-w-2xl text-xs text-destructive">{run.errorMessage}</p>}
        </div>

        <div className="flex shrink-0 flex-col items-end gap-1">
          {isQueued && !hasApiKey && (
            <p className="max-w-xs text-right text-[11px] text-destructive">
              No Anthropic API key configured.{" "}
              <Link to="/settings" className="underline">
                Add one in Settings
              </Link>{" "}
              before starting this run.
            </p>
          )}
          {isQueued && (
            <Button onClick={() => void handleStart()} disabled={isStarting || !hasApiKey}>
              {isStarting ? "Starting…" : "Start"}
            </Button>
          )}
          {isRunning && (
            <Button variant="destructive" onClick={() => void handleStop()} disabled={isStopping}>
              <Square className="mr-1.5 h-3.5 w-3.5" />
              {isStopping ? "Stopping…" : "Stop"}
            </Button>
          )}
        </div>
      </div>

      {actionError && (
        <div className="rounded-md border border-destructive/40 bg-destructive/10 px-2 py-1.5 text-xs text-destructive">
          {actionError}
        </div>
      )}

      <div className="grid flex-1 grid-cols-1 gap-4 overflow-hidden lg:grid-cols-2">
        <div className="flex flex-col gap-2 overflow-hidden">
          <h2 className="text-sm font-medium text-foreground">Activity</h2>
          <div className="flex flex-1 flex-col gap-1.5 overflow-y-auto rounded-md border border-border p-2">
            {activity.length === 0 && !isRunning && (
              <p className="text-xs text-muted-foreground">No activity yet.</p>
            )}
            {activity.length === 0 && isRunning && !streamingText && (
              <p className="text-xs text-muted-foreground">Waiting for the model's first response…</p>
            )}
            {activity.map((event) => (
              <div key={event.id} className="rounded border border-border/60 bg-surface px-2 py-1.5 text-xs">
                <div className="flex items-center justify-between gap-2">
                  <span className="font-medium text-foreground">{event.eventType.replace(/_/g, " ")}</span>
                  <span className="text-[10px] text-subtle-foreground">
                    {new Date(event.createdAt).toLocaleTimeString()}
                  </span>
                </div>
                <p className="mt-0.5 whitespace-pre-wrap text-muted-foreground">
                  {formatActivitySummary(event.payloadJson)}
                </p>
              </div>
            ))}
            {isRunning && streamingText && (
              <div className="rounded border border-primary/40 bg-primary/5 px-2 py-1.5 text-xs">
                <span className="font-medium text-foreground">thinking…</span>
                <p className="mt-0.5 whitespace-pre-wrap text-muted-foreground">{streamingText}</p>
              </div>
            )}
          </div>
        </div>

        <div className="flex flex-col gap-2 overflow-hidden">
          <h2 className="text-sm font-medium text-foreground">Tool calls</h2>
          <div className="flex flex-1 flex-col gap-1.5 overflow-y-auto rounded-md border border-border p-2">
            {toolCalls.length === 0 && (
              <p className="text-xs text-muted-foreground">
                {isTerminal ? "No tool calls — the model answered without using any tools." : "No tool calls yet."}
              </p>
            )}
            {toolCalls.map((call) => {
              const badge = TOOL_CALL_BADGE[call.status];
              return (
                <div key={call.id} className="rounded border border-border/60 bg-surface px-2 py-1.5 text-xs">
                  <div className="flex items-center justify-between gap-2">
                    <span className="font-mono font-medium text-foreground">{call.toolName}</span>
                    <div className="flex items-center gap-1.5">
                      {call.durationMs !== null && (
                        <span className="text-[10px] text-subtle-foreground">{call.durationMs}ms</span>
                      )}
                      <Badge variant={badge.variant}>{badge.label}</Badge>
                    </div>
                  </div>
                  <pre className="mt-1 overflow-x-auto whitespace-pre-wrap break-words text-[11px] text-muted-foreground">
                    {call.inputJson}
                  </pre>
                  {call.outputJson && (
                    <p className="mt-1 whitespace-pre-wrap break-words text-[11px] text-muted-foreground">
                      {call.outputJson}
                    </p>
                  )}
                </div>
              );
            })}
          </div>
        </div>
      </div>

      {isTerminal && (
        <div className="flex min-h-[240px] flex-1 flex-col gap-2 overflow-hidden">
          <h2 className="text-sm font-medium text-foreground">Changes</h2>
          <RunDiffPanel agentRunId={run.id} />
        </div>
      )}

      {run.status === "completed" && (
        <div className="flex flex-col gap-2 overflow-hidden rounded-md border border-border p-3">
          <h2 className="text-sm font-medium text-foreground">Review</h2>
          <ReviewPanel agentRunId={run.id} />
        </div>
      )}

      {run.status === "completed" && (
        <div className="flex flex-col gap-2 overflow-hidden rounded-md border border-border p-3">
          <h2 className="text-sm font-medium text-foreground">Merge</h2>
          <MergePanel agentRunId={run.id} />
        </div>
      )}

      <div className="flex flex-col gap-2 overflow-hidden rounded-md border border-border p-3">
        <h2 className="text-sm font-medium text-foreground">Visual Regression</h2>
        <VisualRegressionPanel agentRunId={run.id} />
      </div>
    </div>
  );
}
