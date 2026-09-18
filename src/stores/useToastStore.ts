import { create } from "zustand";

export type ToastVariant = "error" | "success" | "info";

export interface Toast {
  id: string;
  variant: ToastVariant;
  title: string;
  description?: string;
}

const AUTO_DISMISS_MS: Record<ToastVariant, number> = {
  error: 8000,
  success: 4000,
  info: 5000,
};

interface ToastState {
  toasts: Toast[];
  pushToast: (toast: Omit<Toast, "id">) => string;
  dismissToast: (id: string) => void;
}

/**
 * Minimal, real toast stack — a small corner notification queue, not a full
 * notification center (that's the bell in `TopBar.tsx`, backed by the
 * `notifications` table). This is for transient, session-only feedback:
 * command failures/successes the user should see right when they happen,
 * even on surfaces that don't already have an inline error banner.
 */
export const useToastStore = create<ToastState>((set, get) => ({
  toasts: [],

  pushToast: (toast) => {
    const id = crypto.randomUUID();
    set((s) => ({ toasts: [...s.toasts, { ...toast, id }] }));
    const timeout = AUTO_DISMISS_MS[toast.variant];
    setTimeout(() => get().dismissToast(id), timeout);
    return id;
  },

  dismissToast: (id) => set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) })),
}));

/** Convenience helpers so call sites don't need to know the store shape. */
export function toastError(title: string, description?: string): void {
  useToastStore.getState().pushToast({ variant: "error", title, description });
}

export function toastSuccess(title: string, description?: string): void {
  useToastStore.getState().pushToast({ variant: "success", title, description });
}

export function toastInfo(title: string, description?: string): void {
  useToastStore.getState().pushToast({ variant: "info", title, description });
}
