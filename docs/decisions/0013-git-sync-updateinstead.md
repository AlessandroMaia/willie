# 0013 — Projects sync with the Windows checkout through git remotes

- **Date:** 2026-08-27
- **Status:** accepted

## Context

Decision 0011 puts the agent's working copy on the distribution's own
ext4 disk, not on `/mnt/*`, because every file operation on DrvFs costs
one to two orders of magnitude more than the same operation on ext4.
That workspace is a second copy of the user's code: whatever the agent
commits there must still reach the checkout under `C:\` the user
already has open in an editor and other tools, and whatever the user
commits there must reach the workspace, without either side losing
history or being overwritten by surprise.

## Options

| Option                              | For                              | Against                                        |
| ------------------------------------ | --------------------------------- | ----------------------------------------------- |
| git clone + push with `updateInstead` | full history; git's own conflict detection; no daemon-side merge logic | the Windows tree must be clean to accept a push |
| rsync or a two-way file sync          | works on any folder, git or not   | no history; a conflicting edit on both sides silently picks one, with no record of the other |
| Mount the ext4 workspace back under `C:\` (a second drive letter or junction) | one copy, no sync step needed     | reintroduces the exact DrvFs cost 0011 measured; every `git status` inside the workspace would cross the 9p boundary again |

## Decision

A project's workspace is an ordinary `git clone` of the Windows
checkout, with the source renamed from `origin` to `windows`, its other
remotes copied over unchanged, and `core.autocrlf=false` set so the
ext4 copy never rewrites line endings. The Windows checkout gets
`receive.denyCurrentBranch=updateInstead`, so a plain `git push` into
it updates its working tree directly instead of being refused as a
push into a checked-out branch — no bare mirror, no second remote
repository to keep alive.

**Send to Windows** is `git push windows HEAD:<branch>`, refused first
if the Windows tree is dirty (`updateInstead` only ever touches a clean
tree, and refusing early gives a better message than letting the push
fail) or on a different branch than the one recorded at add time.
**Update from Windows** is `git fetch windows` followed by a
fast-forward-only merge, refused if the workspace holds commits the
Windows checkout does not — the user sends those first, the daemon
never merges automatically. Both directions leave the untouched side
exactly as it was: the refusal happens before git changes anything.

## Consequences

- The Windows checkout is the single point every sync passes through;
  there is no direct workspace-to-workspace path and none is planned.
- A dirty working tree blocks a push by design, on either side: this
  makes losing uncommitted work impossible through sync, at the cost
  of an explicit refusal instead of a silent merge or overwrite.
- History is real git history on both sides — `git log`, `blame` and
  bisect work the same in the workspace and in the Windows checkout.
- Submodules are not recursed and a `.gitattributes` mentioning lfs is
  flagged in the job log, not handled; both stay out of scope until a
  project that needs them shows up.
- Branch switching, rebasing and conflict resolution stay in the
  terminal, inside the workspace, exactly as before this feature — sync
  moves commits, it does not add a git client to the UI.

## Not decided

Whether a project should ever track more than one branch at a time, and
whether a partial send (cherry-picking a subset of the workspace's
commits) is worth adding once `HEAD:<branch>` proves too coarse for a
real workflow.
