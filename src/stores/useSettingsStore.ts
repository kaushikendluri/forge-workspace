import { create } from "zustand";
import { persist } from "zustand/middleware";
import {
  clearApiKey as clearApiKeyCommand,
  getSetting,
  hasApiKey as hasApiKeyCommand,
  isTauriRuntime,
  setApiKey as setApiKeyCommand,
  setSetting,
} from "@/lib/tauri";

export type Theme = "dark" | "light";

/** Backend `settings` table key for the persisted theme. */
const THEME_SETTING_KEY = "ui.theme";

function isTheme(value: string): value is Theme {
  return value === "dark" || value === "light";
}

function errorMessage(err: unknown): string {
  if (typeof err === "string") return err;
  if (err instanceof Error) return err.message;
  return String(err);
}

/** Applies (or removes) the `.light` class on <html> to match the given theme. */
function applyThemeClass(theme: Theme) {
  if (typeof document === "undefined") return;
  document.documentElement.classList.toggle("light", theme === "light");
}

interface SettingsState {
  theme: Theme;
  setTheme: (theme: Theme) => void;
  toggleTheme: () => void;
  /**
   * Loads `ui.theme` from the backend and reconciles it with the
   * localStorage-persisted value — the backend wins once it's available
   * (SQLite is the real state), but localStorage remains a legitimate
   * fallback for when the backend hasn't been reachable yet. If nothing is
   * stored on the backend yet, seeds it with the current theme so it
   * becomes authoritative from here on.
   */
  loadThemeFromBackend: () => Promise<void>;

  /** Whether an Anthropic API key is currently stored in the OS keychain. */
  hasApiKey: boolean;
  isCheckingApiKey: boolean;
  /** Last error from a `setApiKey`/`clearApiKey` call, for inline display. */
  apiKeyError: string | null;
  /** Refreshes `hasApiKey` from the real `has_api_key` command. */
  checkApiKey: () => Promise<void>;
  /** Stores `key` in the OS keychain. Returns whether it succeeded. */
  setApiKey: (key: string) => Promise<boolean>;
  /** Removes the stored API key, if any. */
  clearApiKey: () => Promise<void>;
}

export const useSettingsStore = create<SettingsState>()(
  persist(
    (set, get) => ({
      theme: "dark",
      setTheme: (theme) => {
        applyThemeClass(theme);
        set({ theme });
        if (isTauriRuntime()) {
          // Best-effort — localStorage (via `persist` below) already has the
          // instant, responsive copy; the backend write just makes it the
          // durable source of truth for next launch.
          void setSetting(THEME_SETTING_KEY, theme).catch(() => {});
        }
      },
      toggleTheme: () => {
        const next: Theme = get().theme === "dark" ? "light" : "dark";
        get().setTheme(next);
      },

      loadThemeFromBackend: async () => {
        if (!isTauriRuntime()) return;
        try {
          const stored = await getSetting(THEME_SETTING_KEY);
          if (stored && isTheme(stored)) {
            if (stored !== get().theme) {
              applyThemeClass(stored);
              set({ theme: stored });
            }
          } else {
            // Nothing persisted on the backend yet — seed it with whatever
            // localStorage (or the default) currently holds.
            void setSetting(THEME_SETTING_KEY, get().theme).catch(() => {});
          }
        } catch {
          // Backend unreachable — localStorage remains the source of truth.
        }
      },

      hasApiKey: false,
      isCheckingApiKey: false,
      apiKeyError: null,

      checkApiKey: async () => {
        if (!isTauriRuntime()) return;
        set({ isCheckingApiKey: true });
        try {
          const has = await hasApiKeyCommand();
          set({ hasApiKey: has });
        } catch {
          set({ hasApiKey: false });
        } finally {
          set({ isCheckingApiKey: false });
        }
      },

      setApiKey: async (key) => {
        set({ apiKeyError: null });
        try {
          await setApiKeyCommand(key);
          set({ hasApiKey: true });
          return true;
        } catch (err) {
          set({ apiKeyError: errorMessage(err) });
          return false;
        }
      },

      clearApiKey: async () => {
        set({ apiKeyError: null });
        try {
          await clearApiKeyCommand();
          set({ hasApiKey: false });
        } catch (err) {
          set({ apiKeyError: errorMessage(err) });
        }
      },
    }),
    {
      name: "forge-settings",
      // Only the theme is a legitimate pre-backend fallback; API key state
      // is never persisted to localStorage (it's not a secret itself, but
      // it belongs solely to the backend/keychain's source of truth).
      partialize: (s) => ({ theme: s.theme }),
      onRehydrateStorage: () => (state) => {
        // Reapply the persisted theme's class once the store rehydrates from
        // localStorage, since the class itself isn't part of persisted state.
        if (state) applyThemeClass(state.theme);
      },
    },
  ),
);
