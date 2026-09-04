import type { ReactNode } from "react";
import { Moon, Sun } from "lucide-react";
import { cn } from "@/lib/utils";
import { useSettingsStore, type Theme } from "@/stores/useSettingsStore";
import { Badge } from "@/components/ui/badge";
import { Separator } from "@/components/ui/separator";

function SettingsSection({
  title,
  description,
  children,
}: {
  title: string;
  description?: string;
  children: ReactNode;
}) {
  return (
    <section className="flex flex-col gap-3">
      <div>
        <h2 className="text-sm font-semibold text-foreground">{title}</h2>
        {description && <p className="mt-0.5 text-xs text-muted-foreground">{description}</p>}
      </div>
      {children}
    </section>
  );
}

function BackendRequiredNotice({ milestone }: { milestone: string }) {
  return (
    <div className="flex items-center gap-2 rounded-md border border-dashed border-border bg-surface px-3 py-2.5 text-sm text-muted-foreground">
      <Badge variant="outline">Requires backend</Badge>
      <span>Coming in {milestone}</span>
    </div>
  );
}

const themeOptions: { value: Theme; label: string; icon: typeof Sun }[] = [
  { value: "dark", label: "Dark", icon: Moon },
  { value: "light", label: "Light", icon: Sun },
];

export function Settings() {
  const theme = useSettingsStore((s) => s.theme);
  const setTheme = useSettingsStore((s) => s.setTheme);

  return (
    <div className="flex max-w-2xl flex-col gap-6">
      <h1 className="text-lg font-semibold text-foreground">Settings</h1>

      <SettingsSection title="Appearance" description="Choose how Forge Workspace looks.">
        <div className="flex gap-2">
          {themeOptions.map((option) => (
            <button
              key={option.value}
              type="button"
              onClick={() => setTheme(option.value)}
              className={cn(
                "flex items-center gap-2 rounded-md border px-3 py-2 text-sm font-medium transition-colors",
                theme === option.value
                  ? "border-primary bg-primary/10 text-foreground"
                  : "border-border text-muted-foreground hover:bg-surface-hover",
              )}
              aria-pressed={theme === option.value}
            >
              <option.icon className="h-4 w-4" />
              {option.label}
            </button>
          ))}
        </div>
      </SettingsSection>

      <Separator />

      <SettingsSection
        title="API Keys"
        description="Store provider API keys securely via the OS keychain."
      >
        <BackendRequiredNotice milestone="M4" />
      </SettingsSection>

      <Separator />

      <SettingsSection
        title="Model Configuration"
        description="Choose default models and output limits for agent runs."
      >
        <BackendRequiredNotice milestone="M4" />
      </SettingsSection>
    </div>
  );
}
