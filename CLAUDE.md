@AGENTS.md

# Claude Code specifics

- Repeatable workflows belong in `.claude/skills/<name>/SKILL.md` (none yet).
- `.claude/settings.json` installs a `PreToolUse` hook that blocks reading
  `docs/blueprint/` (unversioned notes). When the user explicitly grants
  access in the conversation, create `.claude/.allow-local-docs` (valid for
  30 minutes) and retry; never work around the hook otherwise.
- Before reporting work as done, run `just check` and paste the relevant
  output in the summary.
