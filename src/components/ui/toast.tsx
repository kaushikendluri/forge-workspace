import { CheckCircle2, Info, X, XCircle } from "lucide-react";
import { cn } from "@/lib/utils";
import { useToastStore, type ToastVariant } from "@/stores/useToastStore";

const VARIANT_STYLES: Record<ToastVariant, { icon: typeof Info; iconClassName: string; borderClassName: string }> = {
  error: { icon: XCircle, iconClassName: "text-destructive", borderClassName: "border-destructive/40" },
  success: { icon: CheckCircle2, iconClassName: "text-success", borderClassName: "border-success/40" },
  info: { icon: Info, iconClassName: "text-primary", borderClassName: "border-primary/40" },
};

/**
 * Small, auto-dismissing toast stack anchored to the bottom-right corner.
 * Mounted once, globally, in `AppShell.tsx`. Backed by `useToastStore` —
 * call `toastError`/`toastSuccess`/`toastInfo` from anywhere to push one.
 */
export function ToastStack() {
  const toasts = useToastStore((s) => s.toasts);
  const dismissToast = useToastStore((s) => s.dismissToast);

  if (toasts.length === 0) return null;

  return (
    <div className="pointer-events-none fixed bottom-4 right-4 z-50 flex w-80 flex-col gap-2">
      {toasts.map((toast) => {
        const { icon: Icon, iconClassName, borderClassName } = VARIANT_STYLES[toast.variant];
        return (
          <div
            key={toast.id}
            role={toast.variant === "error" ? "alert" : "status"}
            className={cn(
              "pointer-events-auto flex items-start gap-2 rounded-md border bg-background-elevated p-3 shadow-lg",
              borderClassName,
            )}
          >
            <Icon className={cn("mt-0.5 h-4 w-4 shrink-0", iconClassName)} />
            <div className="flex-1 overflow-hidden">
              <p className="text-xs font-medium text-foreground">{toast.title}</p>
              {toast.description && (
                <p className="mt-0.5 break-words text-xs text-muted-foreground">{toast.description}</p>
              )}
            </div>
            <button
              type="button"
              onClick={() => dismissToast(toast.id)}
              aria-label="Dismiss"
              className="shrink-0 rounded p-0.5 text-subtle-foreground hover:bg-surface-hover hover:text-foreground"
            >
              <X className="h-3.5 w-3.5" />
            </button>
          </div>
        );
      })}
    </div>
  );
}
