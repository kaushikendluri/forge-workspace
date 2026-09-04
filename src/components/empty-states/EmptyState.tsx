import type { LucideIcon } from "lucide-react";
import { cn } from "@/lib/utils";

export interface EmptyStateAction {
  label: string;
  onClick?: () => void;
  disabled?: boolean;
  /** Shown as a tooltip-less inline hint when the action is disabled (no @radix tooltip dependency here). */
  disabledReason?: string;
  variant?: "primary" | "secondary";
}

export interface EmptyStateProps {
  icon?: LucideIcon;
  title: string;
  description?: string;
  actions?: EmptyStateAction[];
  className?: string;
}

/**
 * Standard empty-state block used across routes that have no backend-backed
 * data yet (Dashboard, Projects, Agents/Tasks/Activity nav targets, etc).
 * Never fabricates data — always an honest "nothing here" message.
 */
export function EmptyState({ icon: Icon, title, description, actions, className }: EmptyStateProps) {
  return (
    <div
      className={cn(
        "flex flex-1 flex-col items-center justify-center gap-3 rounded-lg border border-dashed border-border px-6 py-16 text-center",
        className,
      )}
    >
      {Icon && (
        <div className="mb-1 flex h-10 w-10 items-center justify-center rounded-md bg-surface text-subtle-foreground">
          <Icon className="h-5 w-5" />
        </div>
      )}
      <h3 className="text-sm font-medium text-foreground">{title}</h3>
      {description && (
        <p className="max-w-sm text-sm text-muted-foreground">{description}</p>
      )}
      {actions && actions.length > 0 && (
        <div className="mt-2 flex flex-col items-center gap-2 sm:flex-row">
          {actions.map((action) => (
            <div key={action.label} className="flex flex-col items-center gap-1">
              <button
                type="button"
                onClick={action.onClick}
                disabled={action.disabled}
                className={cn(
                  "inline-flex h-8 items-center justify-center rounded-md px-3 text-xs font-medium transition-colors disabled:cursor-not-allowed disabled:opacity-50",
                  action.variant === "secondary"
                    ? "border border-border bg-transparent text-foreground hover:bg-surface-hover"
                    : "bg-primary text-primary-foreground hover:bg-primary/90",
                )}
              >
                {action.label}
              </button>
              {action.disabled && action.disabledReason && (
                <span className="text-[11px] text-subtle-foreground">{action.disabledReason}</span>
              )}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
