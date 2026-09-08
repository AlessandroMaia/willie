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
import { EDITABLE_FRAGMENTS } from "@/lib/domain/profiles";
import type { Problem } from "@/lib/ipc";
import { profiles } from "@/lib/ipc";
import { asProblem } from "@/lib/problem";
import type { ProfileSummary } from "@/lib/proto";

type SyncAction = "remote" | "push" | "pull";

interface ProfileFragmentsProps {
  name: string;
}

/**
 * A selected profile's own fragment editors and sync controls — the
 * store side of the profiles plugin. Applying a profile to a project
 * lives on `apply-panel.tsx` instead, mounted on that system's own
 * Profiles screen; everything here resets whenever `name` changes,
 * since a different profile shares none of it.
 */
function ProfileFragments({ name }: ProfileFragmentsProps) {
  const [fragmentContent, setFragmentContent] = useState<
    Record<string, string>
  >({});
  const [fragmentBusy, setFragmentBusy] = useState<string | null>(null);
  const [fragmentProblem, setFragmentProblem] = useState<Problem | null>(null);

  const [remoteUrl, setRemoteUrl] = useState("");
  const [syncBusy, setSyncBusy] = useState<SyncAction | null>(null);
  const [syncProblem, setSyncProblem] = useState<Problem | null>(null);
  const [syncMessage, setSyncMessage] = useState<string | null>(null);

  useEffect(() => {
    setFragmentProblem(null);
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
 * The profiles plugin's store screen: list profiles and create one,
 * then edit a selected profile's fragments and sync it to a remote.
 * Applying a profile to a project is out of scope here — that is
 * `apply-panel.tsx`, scoped to one system instead. Reaches the daemon
 * only through `profile.*` (the `profiles` bridge in `lib/ipc.ts`) —
 * this module never imports a feature.
 */
export function ProfileStorePanel() {
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

      {selected && <ProfileFragments name={selected} />}
    </div>
  );
}
