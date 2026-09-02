import { PlusIcon, Trash2Icon } from "lucide-react";
import { useState } from "react";
import { ProblemAlert } from "@/components/problem-alert";
import { StatusBadge } from "@/components/status-badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Field,
  FieldContent,
  FieldDescription,
  FieldLabel,
} from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { Item, ItemContent, ItemGroup } from "@/components/ui/item";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import type { Problem } from "@/lib/ipc";
import type {
  CapabilityInfo,
  ExtraPath,
  Project,
  SandboxProfile,
} from "@/lib/proto";

interface SandboxDialogProps {
  project: Project | null;
  catalogue?: CapabilityInfo[];
  problem: Problem | null;
  onSave: (profile: SandboxProfile) => void;
  onCancel: () => void;
}

/* Every capability but the list one is a boolean override on the
 * profile. No fixed name list duplicates `Capability::is_implemented`
 * here: which rows exist, their order and which are editable all come
 * from the `catalogue` prop alone (`Capability::ALL` order — shipped
 * first, deferred after), so a capability the enforcement plan later
 * implements needs no change in this file to become editable. */
type BooleanCapability = Exclude<CapabilityInfo["capability"], "extra_paths">;

/* This element exists only to carry hover/focus for a deferred row's
 * tooltip trigger, wrapped around its disabled control; it takes no
 * other keyboard action itself. Mirrors `app-sidebar.tsx`'s planned
 * entries: a disabled control receives no pointer or focus events, so
 * the tooltip trigger cannot be the disabled control itself. */
// biome-ignore lint/a11y/noNoninteractiveTabindex: see comment above
const deferredRowTrigger = <div className="block" tabIndex={0} />;

export function SandboxDialog({
  project,
  catalogue = [],
  problem,
  onSave,
  onCancel,
}: SandboxDialogProps) {
  const [local, setLocal] = useState<SandboxProfile>(
    () => project?.sandbox ?? {},
  );

  function setFlag(capability: BooleanCapability, value: boolean) {
    setLocal((prev) => ({ ...prev, [capability]: value }));
  }

  function setPaths(paths: ExtraPath[]) {
    setLocal((prev) => ({ ...prev, extra_paths: paths }));
  }

  function addPath() {
    setPaths([...(local.extra_paths ?? []), { path: "", mode: "ro" }]);
  }

  function removePath(index: number) {
    setPaths((local.extra_paths ?? []).filter((_, i) => i !== index));
  }

  function updatePath(index: number, patch: Partial<ExtraPath>) {
    setPaths(
      (local.extra_paths ?? []).map((entry, i) =>
        i === index ? { ...entry, ...patch } : entry,
      ),
    );
  }

  return (
    <Dialog
      open={project !== null}
      onOpenChange={(open) => {
        if (!open) onCancel();
      }}
    >
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Sandbox for “{project?.name}”</DialogTitle>
          <DialogDescription>
            What this project's sessions may reach beyond the project itself. A
            session records the policy it ran under.
          </DialogDescription>
        </DialogHeader>

        <div className="flex flex-col gap-3">
          {catalogue.map((info) => {
            if (info.capability === "extra_paths") {
              return (
                <div key="extra_paths" className="flex flex-col gap-2">
                  <FieldLabel>{info.display_name}</FieldLabel>
                  <FieldDescription>{info.consequence}</FieldDescription>
                  <ItemGroup className="gap-1">
                    {/* Rows have no identity of their own: they are only
                     * appended and removed, never reordered, so the
                     * index is stable for the row a user is editing. */}
                    {(local.extra_paths ?? []).map((entry, index) => {
                      const modeId = `sandbox-extra-path-${index}-mode`;
                      return (
                        // biome-ignore lint/suspicious/noArrayIndexKey: see comment above
                        <Item key={index} size="xs" variant="outline">
                          <ItemContent className="flex-row items-center gap-2">
                            <Input
                              value={entry.path}
                              onChange={(e) =>
                                updatePath(index, { path: e.target.value })
                              }
                              placeholder="/srv/shared"
                              aria-label={`extra path ${index + 1}`}
                            />
                            <Field
                              orientation="horizontal"
                              className="w-fit whitespace-nowrap"
                            >
                              <Checkbox
                                id={modeId}
                                checked={entry.mode === "rw"}
                                onCheckedChange={(value) =>
                                  updatePath(index, {
                                    mode: value === true ? "rw" : "ro",
                                  })
                                }
                              />
                              <FieldLabel htmlFor={modeId}>
                                read-write
                              </FieldLabel>
                            </Field>
                            <Button
                              variant="ghost"
                              size="icon-sm"
                              aria-label="Remove path"
                              onClick={() => removePath(index)}
                            >
                              <Trash2Icon />
                            </Button>
                          </ItemContent>
                        </Item>
                      );
                    })}
                  </ItemGroup>
                  <div>
                    <Button variant="outline" size="sm" onClick={addPath}>
                      <PlusIcon /> Add path
                    </Button>
                  </div>
                </div>
              );
            }

            const capability = info.capability;
            const id = `sandbox-${capability}`;
            const forced = capability === "project_rw";
            /* An absent override means "whatever the harness decided",
             * and only the catalogue knows what that is: the trait's
             * default leaves `agent.state` off where Claude Code turns
             * it on, so a guess here would show a credential as
             * mounted for a harness that never asked for it. */
            const checked = forced
              ? true
              : info.implemented
                ? (local[capability] ?? info.default_enabled)
                : false;

            const content = (
              <>
                <Checkbox
                  id={id}
                  checked={checked}
                  disabled={forced || !info.implemented}
                  onCheckedChange={(value) =>
                    setFlag(capability, value === true)
                  }
                />
                <FieldContent>
                  <div className="flex items-center gap-2">
                    <FieldLabel htmlFor={id}>{info.display_name}</FieldLabel>
                    {!info.implemented && (
                      <StatusBadge tone="muted">not available</StatusBadge>
                    )}
                  </div>
                  <FieldDescription>{info.consequence}</FieldDescription>
                </FieldContent>
              </>
            );

            if (info.implemented) {
              return (
                <Field key={capability} orientation="horizontal">
                  {content}
                </Field>
              );
            }

            return (
              <Tooltip key={capability}>
                <TooltipTrigger render={deferredRowTrigger}>
                  <Field
                    orientation="horizontal"
                    className="pointer-events-none"
                  >
                    {content}
                  </Field>
                </TooltipTrigger>
                <TooltipContent side="right">
                  This version cannot apply it yet
                </TooltipContent>
              </Tooltip>
            );
          })}
        </div>

        {problem && <ProblemAlert problem={problem} />}

        <DialogFooter>
          <Button variant="ghost" onClick={onCancel}>
            Cancel
          </Button>
          <Button onClick={() => onSave(local)}>Save</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
