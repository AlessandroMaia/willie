import { useEffect, useState } from "react";
import { ProblemAlert } from "@/components/problem-alert";
import { Button } from "@/components/ui/button";
import { Field, FieldLabel } from "@/components/ui/field";
import {
  Item,
  ItemContent,
  ItemDescription,
  ItemGroup,
  ItemTitle,
} from "@/components/ui/item";
import { Spinner } from "@/components/ui/spinner";
import { summarizeChanges } from "@/lib/domain/profiles";
import type { Problem } from "@/lib/ipc";
import { plugins, profiles } from "@/lib/ipc";
import { asProblem } from "@/lib/problem";
import type { Change, ProfileSummary } from "@/lib/proto";

/** Which of the panel's own async actions is in flight, so a button can
 * show its own spinner without a second boolean per control. */
type ApplyAction = "enable" | "check" | "apply";

interface ApplyPanelProps {
  /** The system this panel applies a profile to. The panel never asks
   * `useCurrentSystem()` itself — the feature that mounts it decides. */
  projectId: string;
}

/**
 * The profiles plugin's apply flow for one system: pick a profile from
 * the store, enable the plugin for this project if it is not already,
 * check what applying it would change, then confirm. Reaches the
 * daemon only through `profile.*`/`plugin.enable` — this module never
 * imports a feature.
 */
export function ApplyPanel({ projectId }: ApplyPanelProps) {
  const [list, setList] = useState<ProfileSummary[]>([]);
  const [listProblem, setListProblem] = useState<Problem | null>(null);
  const [profileName, setProfileName] = useState("");
  const [checkChanges, setCheckChanges] = useState<Change[] | null>(null);
  const [backupPath, setBackupPath] = useState<string | null>(null);
  const [busy, setBusy] = useState<ApplyAction | null>(null);
  const [problem, setProblem] = useState<Problem | null>(null);

  useEffect(() => {
    profiles
      .list()
      .then(setList)
      .catch((error: unknown) => setListProblem(asProblem(error)));
  }, []);

  async function enableForSystem() {
    setBusy("enable");
    setProblem(null);
    try {
      await plugins.enable("profile", projectId);
    } catch (error) {
      setProblem(asProblem(error));
    } finally {
      setBusy(null);
    }
  }

  async function runCheck() {
    setBusy("check");
    setProblem(null);
    setBackupPath(null);
    try {
      const result = await profiles.check(profileName, projectId);
      setCheckChanges(result.changes);
    } catch (error) {
      setCheckChanges(null);
      setProblem(asProblem(error));
    } finally {
      setBusy(null);
    }
  }

  async function confirmApply() {
    setBusy("apply");
    setProblem(null);
    try {
      const result = await profiles.apply(profileName, projectId);
      setCheckChanges(result.changes);
      setBackupPath(result.backup_path);
    } catch (error) {
      setProblem(asProblem(error));
    } finally {
      setBusy(null);
    }
  }

  const counts = checkChanges ? summarizeChanges(checkChanges) : null;

  return (
    <div className="flex flex-col gap-4">
      {listProblem && <ProblemAlert problem={listProblem} />}
      {problem && <ProblemAlert problem={problem} />}

      <Field className="max-w-xs">
        <FieldLabel htmlFor="apply-profile">Profile</FieldLabel>
        <select
          id="apply-profile"
          className="h-8 w-full rounded-lg border border-input bg-transparent px-2.5 text-sm outline-none focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50"
          value={profileName}
          onChange={(e) => {
            setProfileName(e.target.value);
            setCheckChanges(null);
            setBackupPath(null);
          }}
        >
          <option value="">Choose a profile…</option>
          {list.map((profile) => (
            <option key={profile.name} value={profile.name}>
              {profile.name}
            </option>
          ))}
        </select>
      </Field>

      <div className="flex gap-2">
        <Button
          size="sm"
          variant="outline"
          disabled={busy !== null}
          onClick={() => void enableForSystem()}
        >
          {busy === "enable" && <Spinner />} Enable profiles for this system
        </Button>
        <Button
          size="sm"
          disabled={profileName === "" || busy !== null}
          onClick={() => void runCheck()}
        >
          {busy === "check" && <Spinner />} Check
        </Button>
      </div>

      {checkChanges && counts && (
        <div className="flex flex-col gap-2">
          <ItemDescription>
            {counts.create} to create, {counts.merge} to merge,{" "}
            {counts.overwrite} to overwrite
          </ItemDescription>
          <ItemGroup className="gap-1">
            {checkChanges.map((change) => (
              <Item key={change.path} variant="outline">
                <ItemContent>
                  <ItemTitle>{change.path}</ItemTitle>
                  <ItemDescription>{change.kind}</ItemDescription>
                  <ItemDescription className="whitespace-pre-wrap font-mono text-xs">
                    {change.after.slice(0, 200)}
                  </ItemDescription>
                </ItemContent>
              </Item>
            ))}
          </ItemGroup>
          {backupPath === null && (
            <Button
              size="sm"
              className="w-fit"
              disabled={busy !== null}
              onClick={() => void confirmApply()}
            >
              {busy === "apply" && <Spinner />} Confirm & apply
            </Button>
          )}
        </div>
      )}

      {backupPath && (
        <ItemDescription>Applied. Backup saved at {backupPath}</ItemDescription>
      )}
    </div>
  );
}
