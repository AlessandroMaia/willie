import { Button } from "@/components/ui/button";
import { ButtonGroup } from "@/components/ui/button-group";
import { Field, FieldLabel } from "@/components/ui/field";
import { type ThemeMode, useThemeMode } from "@/store/use-theme";

const MODES: { value: ThemeMode; label: string }[] = [
  { value: "system", label: "System" },
  { value: "light", label: "Light" },
  { value: "dark", label: "Dark" },
];

/** The machine-wide Settings screen: today, only the theme. */
export function SettingsScreen() {
  const { mode, setMode } = useThemeMode();

  return (
    <div className="mx-auto flex max-w-md flex-col gap-6">
      <header>
        <h1 className="font-semibold text-lg">Settings</h1>
      </header>

      <Field>
        <FieldLabel>Theme</FieldLabel>
        <ButtonGroup>
          {MODES.map((m) => (
            <Button
              key={m.value}
              variant={mode === m.value ? "secondary" : "outline"}
              aria-pressed={mode === m.value}
              onClick={() => setMode(m.value)}
            >
              {m.label}
            </Button>
          ))}
        </ButtonGroup>
      </Field>
    </div>
  );
}
