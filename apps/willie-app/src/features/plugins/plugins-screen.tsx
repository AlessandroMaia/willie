import { useCallback, useEffect, useState } from "react";
import { ProblemAlert } from "@/components/problem-alert";
import { StatusBadge } from "@/components/status-badge";
import { Checkbox } from "@/components/ui/checkbox";
import { Field, FieldLabel } from "@/components/ui/field";
import {
  Item,
  ItemActions,
  ItemContent,
  ItemDescription,
  ItemGroup,
  ItemTitle,
} from "@/components/ui/item";
import type { Problem } from "@/lib/ipc";
import { plugins } from "@/lib/ipc";
import { asProblem } from "@/lib/problem";
import type { PluginStatus } from "@/lib/proto";
import { ProfilesPanel } from "@/plugins/profiles/profiles-panel";
import { useSnapshot } from "@/store/use-snapshot";

/** The profiles plugin's real id (singular — `manifest().id` in
 * `crates/willie-plugins/profiles`), never "profiles": the panel mounts
 * strictly on this literal. */
const PROFILES_PLUGIN_ID = "profile";

function scopeLabel(status: PluginStatus): string {
  return status.scope === "global" ? "Global" : "Per-project";
}

/* A `Global` plugin's `enabled` is a bare flag this screen can flip
 * directly. A `PerProject` plugin's is the set of projects it runs in,
 * decided on each project's own surface (§ the profiles panel) — this
 * screen only reports that fact, it offers no control for it here. */
function isGloballyEnabled(status: PluginStatus): boolean {
  return "global" in status.enabled && status.enabled.global;
}

/**
 * Lists every compiled-in plugin: its name, scope, and — for a
 * `Global` plugin — a toggle; a `PerProject` plugin gets a note
 * instead, since its enablement lives per project rather than here.
 * `plugin.list` is the source of truth (also carried on the daemon
 * snapshot, but that copy only refreshes on a full resnapshot or a
 * `PluginChanged` event, which the daemon does not emit yet); the
 * snapshot subscription here only keeps the daemon connection warm,
 * the same way every other daemon-backed screen does.
 */
export function PluginsScreen() {
  useSnapshot();
  const [list, setList] = useState<PluginStatus[]>([]);
  const [problem, setProblem] = useState<Problem | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);

  const load = useCallback(() => {
    plugins
      .list()
      .then(setList)
      .catch((error: unknown) => setProblem(asProblem(error)));
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  const profilesPlugin = list.find((p) => p.id === PROFILES_PLUGIN_ID);

  async function toggle(status: PluginStatus, next: boolean) {
    setBusyId(status.id);
    setProblem(null);
    try {
      const updated = next
        ? await plugins.enable(status.id)
        : await plugins.disable(status.id);
      setList((current) =>
        current.map((p) => (p.id === updated.id ? updated : p)),
      );
    } catch (error) {
      setProblem(asProblem(error));
    } finally {
      setBusyId(null);
    }
  }

  return (
    <div className="mx-auto flex max-w-3xl flex-col gap-6">
      <header>
        <h1 className="font-semibold text-lg">Plugins</h1>
      </header>

      {problem && <ProblemAlert problem={problem} />}

      <ItemGroup className="gap-1">
        {list.map((status) => {
          const fieldId = `plugin-${status.id}-enabled`;
          return (
            <Item key={status.id} variant="outline">
              <ItemContent>
                <ItemTitle>{status.name}</ItemTitle>
                <ItemDescription>{scopeLabel(status)}</ItemDescription>
                {status.degraded && (
                  <StatusBadge tone="error" className="w-fit">
                    error
                  </StatusBadge>
                )}
              </ItemContent>
              <ItemActions>
                {status.scope === "global" ? (
                  <Field orientation="horizontal">
                    <Checkbox
                      id={fieldId}
                      checked={isGloballyEnabled(status)}
                      disabled={busyId !== null}
                      onCheckedChange={(checked) =>
                        toggle(status, checked === true)
                      }
                    />
                    <FieldLabel htmlFor={fieldId}>Enabled</FieldLabel>
                  </Field>
                ) : (
                  <ItemDescription>Enabled per project</ItemDescription>
                )}
              </ItemActions>
            </Item>
          );
        })}
      </ItemGroup>

      {profilesPlugin && !profilesPlugin.degraded && <ProfilesPanel />}
    </div>
  );
}
