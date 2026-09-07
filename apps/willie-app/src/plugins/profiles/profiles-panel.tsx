import { useCallback, useEffect, useState } from "react";
import { FailureChip } from "@/components/failure-chip";
import { ProblemAlert } from "@/components/problem-alert";
import { Button } from "@/components/ui/button";
import { Field, FieldLabel } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import {
  Item,
  ItemActions,
  ItemContent,
  ItemDescription,
  ItemGroup,
  ItemTitle,
} from "@/components/ui/item";
import { Spinner } from "@/components/ui/spinner";
import { EDITABLE_FRAGMENTS, summarizeChanges } from "@/lib/domain/profiles";
import type { Problem } from "@/lib/ipc";
import { plugins, profiles } from "@/lib/ipc";
import { asProblem } from "@/lib/problem";
import type { Change, ProfileSummary, Project } from "@/lib/proto";
import { useSnapshot } from "@/store/use-snapshot";

/** Which of the panel's own async actions is in flight, so a button can
 * show its own spinner without a second boolean per control. */
type ApplyAction = "enable" | "check" | "apply";
type SyncAction = "remote" | "push" | "pull";

interface ProfileDetailProps {
  name: string;
  projects: Project[];
}

/**
 * Everything scoped to one selected profile: its fragment editors, the
 * apply-to-project flow, and the sync controls. Kept apart from the
 * list/create section above it because every piece of state here resets
 * when `name` changes — a different profile shares none of it.
 */
function ProfileDetail({ name, projects }: ProfileDetailProps) {
  const [fragmentContent, setFragmentContent] = useState<
    Record<string, string>
  >({});
  const [fragmentBusy, setFragmentBusy] = useState<string | null>(null);
  const [fragmentProblem, setFragmentProblem] = useState<Problem | null>(null);

  const [projectId, setProjectId] = useState("");
  const [checkChanges, setCheckChanges] = useState<Change[] | null>(null);
  const [backupPath, setBackupPath] = useState<string | null>(null);
  const [applyBusy, setApplyBusy] = useState<ApplyAction | null>(null);
  const [applyProblem, setApplyProblem] = useState<Problem | null>(null);

  const [remoteUrl, setRemoteUrl] = useState("");
  const [syncBusy, setSyncBusy] = useState<SyncAction | null>(null);
  const [syncProblem, setSyncProblem] = useState<Problem | null>(null);
  const [syncMessage, setSyncMessage] = useState<string | null>(null);

  useEffect(() => {
    setFragmentProblem(null);
    setProjectId("");
    setCheckChanges(null);
    setBackupPath(null);
    setApplyProblem(null);
    setRemoteUrl("");
    setSyncProblem(null);
    setSyncMessage(null);

    let cancelled = false;
    Promise.all(
      EDITABLE_FRAGMENTS.map((fragment) =>
        profiles
          .readFragment(name, fragment.id)
          .then((r) => [fragment.id, r.content] as const),
      ),
    )
      .then((entries) => {
        if (!cancelled) setFragmentContent(Object.fromEntries(entries));
      })
      .catch((error: unknown) => {
        if (!cancelled) setFragmentProblem(asProblem(error));
      });
    return () => {
      cancelled = true;
    };
  }, [name]);

  async function saveFragment(fragmentId: string) {
    setFragmentBusy(fragmentId);
    setFragmentProblem(null);
    try {
      const content = fragmentContent[fragmentId] ?? "";
      const saved = await profiles.writeFragment(name, fragmentId, content);
      setFragmentContent((current) => ({
        ...current,
        [fragmentId]: saved.content,
      }));
    } catch (error) {
      setFragmentProblem(asProblem(error));
    } finally {
      setFragmentBusy(null);
    }
  }

  async function runCheck() {
    setApplyBusy("check");
    setApplyProblem(null);
    setBackupPath(null);
    try {
      const result = await profiles.check(name, projectId);
      setCheckChanges(result.changes);
    } catch (error) {
      setCheckChanges(null);
      setApplyProblem(asProblem(error));
    } finally {
      setApplyBusy(null);
    }
  }

  async function confirmApply() {
    setApplyBusy("apply");
    setApplyProblem(null);
    try {
      const result = await profiles.apply(name, projectId);
      setCheckChanges(result.changes);
      setBackupPath(result.backup_path);
    } catch (error) {
      setApplyProblem(asProblem(error));
    } finally {
      setApplyBusy(null);
    }
  }

  async function enableForProject() {
    setApplyBusy("enable");
    setApplyProblem(null);
    try {
      await plugins.enable("profile", projectId);
    } catch (error) {
      setApplyProblem(asProblem(error));
    } finally {
      setApplyBusy(null);
    }
  }

  async function submitRemote() {
    setSyncBusy("remote");
    setSyncProblem(null);
    setSyncMessage(null);
    try {
      await profiles.setRemote(name, remoteUrl);
      setSyncMessage("Remote set.");
    } catch (error) {
      setSyncProblem(asProblem(error));
    } finally {
      setSyncBusy(null);
    }
  }

  async function runPush() {
    setSyncBusy("push");
    setSyncProblem(null);
    setSyncMessage(null);
    try {
      await profiles.push(name);
      setSyncMessage("Pushed.");
    } catch (error) {
      setSyncProblem(asProblem(error));
    } finally {
      setSyncBusy(null);
    }
  }

  async function runPull() {
    setSyncBusy("pull");
    setSyncProblem(null);
    setSyncMessage(null);
    try {
      await profiles.pull(name);
      setSyncMessage("Pulled.");
    } catch (error) {
      setSyncProblem(asProblem(error));
    } finally {
      setSyncBusy(null);
    }
  }

  const counts = checkChanges ? summarizeChanges(checkChanges) : null;

  return (
    <div className="flex flex-col gap-6 border-t pt-4">
      <section className="flex flex-col gap-4">
        <h3 className="font-medium text-sm">Fragments</h3>
        {fragmentProblem && <ProblemAlert problem={fragmentProblem} />}
        {EDITABLE_FRAGMENTS.map((fragment) => (
          <Field key={fragment.id}>
            <FieldLabel htmlFor={`fragment-${fragment.id}`}>
              {fragment.label}
            </FieldLabel>
            <textarea
              id={`fragment-${fragment.id}`}
              className="min-h-28 w-full rounded-lg border border-input bg-transparent p-2.5 font-mono text-sm outline-none focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50"
              value={fragmentContent[fragment.id] ?? ""}
              onChange={(e) =>
                setFragmentContent((current) => ({
                  ...current,
                  [fragment.id]: e.target.value,
                }))
              }
            />
            <Button
              size="sm"
              className="w-fit"
              aria-label={`Save ${fragment.label}`}
              disabled={fragmentBusy !== null}
              onClick={() => void saveFragment(fragment.id)}
            >
              {fragmentBusy === fragment.id && <Spinner />} Save
            </Button>
          </Field>
        ))}
      </section>

      <section className="flex flex-col gap-3">
        <h3 className="font-medium text-sm">Apply to project</h3>
        {applyProblem && <ProblemAlert problem={applyProblem} />}
        <Field className="max-w-xs">
          <FieldLabel htmlFor="apply-project">Project</FieldLabel>
          <select
            id="apply-project"
            className="h-8 w-full rounded-lg border border-input bg-transparent px-2.5 text-sm outline-none focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50"
            value={projectId}
            onChange={(e) => {
              setProjectId(e.target.value);
              setCheckChanges(null);
              setBackupPath(null);
            }}
          >
            <option value="">Choose a project…</option>
            {projects.map((project) => (
              <option key={project.id} value={project.id}>
                {project.name}
              </option>
            ))}
          </select>
        </Field>

        <div className="flex gap-2">
          <Button
            size="sm"
            variant="outline"
            disabled={projectId === "" || applyBusy !== null}
            onClick={() => void enableForProject()}
          >
            {applyBusy === "enable" && <Spinner />} Enable profiles for this
            project
          </Button>
          <Button
            size="sm"
            disabled={projectId === "" || applyBusy !== null}
            onClick={() => void runCheck()}
          >
            {applyBusy === "check" && <Spinner />} Check
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
                disabled={applyBusy !== null}
                onClick={() => void confirmApply()}
              >
                {applyBusy === "apply" && <Spinner />} Confirm & apply
              </Button>
            )}
          </div>
        )}

        {backupPath && (
          <ItemDescription>
            Applied. Backup saved at {backupPath}
          </ItemDescription>
        )}
      </section>

      <section className="flex flex-col gap-3">
        <h3 className="font-medium text-sm">Sync</h3>
        {syncProblem && (
          <FailureChip
            code={syncProblem.code}
            message={syncProblem.message}
            remediation={syncProblem.remediation}
          />
        )}
        {syncMessage && <ItemDescription>{syncMessage}</ItemDescription>}
        <Field orientation="horizontal" className="max-w-md">
          <FieldLabel htmlFor="remote-url">Remote URL</FieldLabel>
          <Input
            id="remote-url"
            value={remoteUrl}
            placeholder="git@host:path.git"
            onChange={(e) => setRemoteUrl(e.target.value)}
          />
          <Button
            size="sm"
            disabled={syncBusy !== null || remoteUrl.trim() === ""}
            onClick={() => void submitRemote()}
          >
            {syncBusy === "remote" && <Spinner />} Set remote
          </Button>
        </Field>
        <div className="flex gap-2">
          <Button
            size="sm"
            variant="outline"
            disabled={syncBusy !== null}
            onClick={() => void runPush()}
          >
            {syncBusy === "push" && <Spinner />} Push
          </Button>
          <Button
            size="sm"
            variant="outline"
            disabled={syncBusy !== null}
            onClick={() => void runPull()}
          >
            {syncBusy === "pull" && <Spinner />} Pull
          </Button>
        </div>
      </section>
    </div>
  );
}

/**
 * The profiles plugin's own screen: list profiles and create one, edit
 * a selected profile's fragments, apply it to a project (with the
 * backup path), and push/pull it to a remote. Everything here reaches
 * the daemon only through `profile.*` (the `profiles` bridge in
 * `lib/ipc.ts`, itself the engine's one guarded `plugin_call`
 * pass-through) — this module never imports a feature.
 */
export function ProfilesPanel() {
  const store = useSnapshot();
  const projects = store.snapshot?.projects ?? [];

  const [list, setList] = useState<ProfileSummary[]>([]);
  const [listProblem, setListProblem] = useState<Problem | null>(null);
  const [newName, setNewName] = useState("");
  const [creating, setCreating] = useState(false);
  const [selected, setSelected] = useState<string | null>(null);

  const loadList = useCallback(() => {
    profiles
      .list()
      .then(setList)
      .catch((error: unknown) => setListProblem(asProblem(error)));
  }, []);

  useEffect(() => {
    loadList();
  }, [loadList]);

  async function createProfile() {
    setCreating(true);
    setListProblem(null);
    try {
      const created = await profiles.create(newName.trim());
      setList((current) =>
        [...current, created].sort((a, b) => a.name.localeCompare(b.name)),
      );
      setNewName("");
      setSelected(created.name);
    } catch (error) {
      setListProblem(asProblem(error));
    } finally {
      setCreating(false);
    }
  }

  return (
    <div className="flex flex-col gap-6">
      <header>
        <h2 className="font-semibold text-base">Profiles</h2>
      </header>

      {listProblem && <ProblemAlert problem={listProblem} />}

      <Field orientation="horizontal" className="max-w-md">
        <FieldLabel htmlFor="new-profile-name">New profile</FieldLabel>
        <Input
          id="new-profile-name"
          value={newName}
          placeholder="name"
          onChange={(e) => setNewName(e.target.value)}
        />
        <Button
          size="sm"
          disabled={creating || newName.trim() === ""}
          onClick={() => void createProfile()}
        >
          {creating && <Spinner />} New profile
        </Button>
      </Field>

      <ItemGroup className="gap-1">
        {list.map((profile) => (
          <Item key={profile.name} variant="outline">
            <ItemContent>
              <ItemTitle>{profile.name}</ItemTitle>
              <ItemDescription>
                {profile.fragments_active.length > 0
                  ? profile.fragments_active.join(", ")
                  : "no fragments yet"}
              </ItemDescription>
            </ItemContent>
            <ItemActions>
              <Button
                size="sm"
                variant={profile.name === selected ? "secondary" : "outline"}
                aria-label={`Select ${profile.name}`}
                onClick={() => setSelected(profile.name)}
              >
                {profile.name === selected ? "Selected" : "Select"}
              </Button>
            </ItemActions>
          </Item>
        ))}
      </ItemGroup>

      {selected && <ProfileDetail name={selected} projects={projects} />}
    </div>
  );
}
