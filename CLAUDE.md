@AGENTS.md

# Claude Code specifics

- **Session start:** read `docs/superpowers/willie-handoff.md` first. Its
  top entry names the active plan under `docs/superpowers/plans/`;
  continue at that plan's first unchecked step and tick steps as they
  land. Before a session ends, bring the handoff up to date: state,
  uncommitted work, the exact next step. Nothing that must survive the
  session lives only in the scratchpad or the conversation.
- Repeatable workflows belong in `.claude/skills/<name>/SKILL.md` (none yet).
- `.claude/settings.json` installs a `PreToolUse` hook that blocks reading
  `docs/blueprint/` (unversioned notes). When the user explicitly grants
  access in the conversation, create `.claude/.allow-local-docs` (valid for
  30 minutes) and retry; never work around the hook otherwise.
- Before reporting work as done, run `just check` and paste the relevant
  output in the summary.
