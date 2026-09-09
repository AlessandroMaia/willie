/* The Tauri v2 bridge, answered by fixtures. Injected before any app
 * script so the frontend renders in a plain browser with no engine, no
 * distribution and no daemon behind it.
 *
 * Every shape here mirrors `apps/willie-app/src/lib/proto.ts` and the
 * engine's own `EngineStatus`. Timestamps are whole-second epochs as
 * strings, the daemon's wire format: an ISO date renders as "unknown"
 * (see `components/relative-time.tsx`). A fixture that drifts from the
 * wire shows up as a UI bug that does not exist, so correct the
 * fixture before reporting anything it produced. */
(() => {
  const now = Date.now();

  /* The daemon's wire format for every timestamp is a whole-second
   * epoch as a string (see components/relative-time.tsx); an ISO date
   * here renders as "unknown". */
  const ago = (m) => String(Math.floor((now - m * 60000) / 1000));

  const cbs = new Map();
  let nextCb = 1;
  const listeners = new Map(); // event name -> [handlerId]

  const project = (id, name, slug, state) => ({
    id,
    name,
    slug,
    source: "D:\\Projects\\" + slug,
    workspace: "/home/willie/projects/" + slug,
    branch: "main",
    state,
    source_present: true,
    created_at: ago(60 * 24 * 3),
    sandbox: { project_rw: true, agent_state: true, tools_ro: true },
  });

  const projects = [
    project("proj_01JAAAAAAAAAAAAAAAAAAAAAAA", "willie", "willie", {
      state: "ready",
    }),
    project("proj_01JBBBBBBBBBBBBBBBBBBBBBBB", "atlas-api", "atlas-api", {
      state: "ready",
    }),
    project(
      "proj_01JCCCCCCCCCCCCCCCCCCCCCCC",
      "legacy-portal",
      "legacy-portal",
      {
        state: "failed",
        code: "workspace_missing",
        message: "The workspace directory no longer exists in the distro.",
        remediation: "Relocate the system or remove it.",
      },
    ),
  ];

  const sandboxState = {
    applied: ["project_rw", "agent_state", "tools_ro"],
    unavailable: ["ssh"],
    degraded: ["caches_rw"],
    denied: [
      {
        class: "syscall",
        name: "ptrace",
        count: 3,
        first_at: ago(40),
        last_at: ago(4),
      },
      {
        class: "terminal",
        name: "network.connect api.github.com",
        count: 12,
        first_at: ago(35),
        last_at: ago(1),
      },
    ],
  };

  const sessions = [
    {
      id: "sess_01JRUNNINGAAAAAAAAAAAAAAAA",
      project_id: projects[0].id,
      harness: "claude-code",
      workspace: projects[0].workspace,
      kind: "agent",
      state: { state: "running" },
      created_at: ago(52),
      started_at: ago(52),
      pid: 4211,
      clients: 1,
      title: "Fix the sidebar collapse animation and ship the release note",
      sandbox: sandboxState,
    },
    {
      id: "sess_01JSHELLAAAAAAAAAAAAAAAAAA",
      project_id: projects[0].id,
      harness: "shell",
      workspace: projects[0].workspace,
      kind: "shell",
      state: { state: "running" },
      created_at: ago(14),
      started_at: ago(14),
      pid: 4290,
      clients: 1,
      label: "build",
      sandbox: sandboxState,
    },
    {
      id: "sess_01JEXITEDAAAAAAAAAAAAAAAAA",
      project_id: projects[0].id,
      harness: "claude-code",
      workspace: projects[0].workspace,
      kind: "agent",
      state: { state: "exited", code: 0, signal: null },
      created_at: ago(300),
      started_at: ago(300),
      finished_at: ago(240),
      clients: 0,
      title: "Rename the profile store fragments",
      sandbox: sandboxState,
    },
    {
      id: "sess_01JFAILEDAAAAAAAAAAAAAAAAA",
      project_id: projects[1].id,
      harness: "claude-code",
      workspace: projects[1].workspace,
      kind: "agent",
      state: {
        state: "failed",
        code: "sandbox_backend_missing",
        message: "bwrap is not available in the distribution.",
        remediation: "Run `willie doctor` and reinstall the distribution.",
      },
      created_at: ago(120),
      started_at: ago(120),
      finished_at: ago(119),
      clients: 0,
      sandbox: sandboxState,
    },
  ];

  const snapshot = {
    seq: 42,
    projects,
    sessions,
    jobs: [
      {
        id: "job_01JADDAAAAAAAAAAAAAAAAAAAA",
        kind: "add",
        project_id: projects[1].id,
        state: { state: "running" },
        started_at: ago(2),
        log_tail: "Copying the working tree into the distro...\n",
      },
    ],
    plugins: [
      {
        id: "profiles",
        name: "Profiles",
        scope: "per_project",
        enabled: { per_project: [projects[0].id] },
        degraded: false,
      },
      {
        id: "usage",
        name: "Usage",
        scope: "global",
        enabled: { global: true },
        degraded: false,
      },
    ],
  };

  const engineStatus = {
    engine_version: "0.1.0",
    wsl: {
      installed: true,
      version: "2.6.1.0",
      meets_minimum: true,
      minimum: "2.0.0",
    },
    distro: {
      registered: true,
      running: true,
      install_dir: "C:\\Users\\dev\\AppData\\Local\\Willie\\distro",
    },
    distro_error: null,
    daemon: { state: "running", willie_version: "0.1.0", image_version: "0.1.0" },
    doctor: {
      checks: [
        {
          name: "wsl.version",
          status: "ok",
          detail: "WSL 2.6.1.0",
          required: true,
        },
        {
          name: "distro.registered",
          status: "ok",
          detail: "willie",
          required: true,
        },
        {
          name: "daemon.socket",
          status: "ok",
          detail: "/run/willie/willied.sock",
          required: true,
        },
        {
          name: "sandbox.bwrap",
          status: "fail",
          detail: "bwrap not found",
          remediation: "Reinstall the distribution image.",
          required: true,
        },
        {
          name: "interop.appendWindowsPath",
          status: "skip",
          detail: "not applicable",
          required: false,
        },
      ],
    },
    image_available: true,
  };

  const capabilities = [
    [
      "project_rw",
      "Project files (read/write)",
      "The agent can change your working tree.",
      true,
      true,
    ],
    [
      "agent_state",
      "Agent state",
      "Conversations and credentials persist between sessions.",
      true,
      true,
    ],
    [
      "tools_ro",
      "Toolchain (read-only)",
      "The agent can run the managed tools.",
      true,
      true,
    ],
    [
      "caches_rw",
      "Caches",
      "Package caches are shared with the host.",
      true,
      false,
    ],
    [
      "git_identity",
      "Git identity",
      "Commits carry your name and e-mail.",
      true,
      false,
    ],
    [
      "extra_paths",
      "Extra paths",
      "Named paths outside the workspace become visible.",
      false,
      false,
    ],
    [
      "home_persistent",
      "Persistent home",
      "The sandbox home survives the session.",
      true,
      false,
    ],
    [
      "ssh",
      "SSH agent",
      "The agent can authenticate to remotes as you.",
      false,
      false,
    ],
    [
      "mnt_all",
      "Windows drives",
      "Every mounted Windows drive is readable.",
      true,
      false,
    ],
    [
      "windows_interop",
      "Windows interop",
      "The agent can launch Windows executables.",
      true,
      false,
    ],
  ].map(
    ([
      capability,
      display_name,
      consequence,
      implemented,
      default_enabled,
    ]) => ({
      capability,
      display_name,
      consequence,
      implemented,
      default_enabled,
    }),
  );

  const results = {
    engine_status: engineStatus,
    engine_doctor: engineStatus.doctor,
    engine_logon_fix_script:
      "wsl.exe --user root -d willie /usr/lib/willie/logon-fix",
    state_snapshot: snapshot,
    ui_prefs: { current_project: projects[0].id },
    set_ui_prefs: null,
    editor_available: true,
    projects_roots: ["D:\\Projects"],
    discover_projects: [
      { path: "D:\\Projects\\marketing-site", name: "marketing-site" },
      { path: "D:\\Projects\\spike-wasm", name: "spike-wasm" },
    ],
    sandbox_catalogue: capabilities,
    tool_list: {
      tools: [
        {
          id: "claude-code",
          name: "Claude Code",
          installed: true,
          version: "2.1.252",
          recorded_version: "2.1.250",
        },
        { id: "node", name: "Node.js", installed: true, version: "24.18.0" },
        { id: "git", name: "Git", installed: true, version: "2.51.0" },
        { id: "codex", name: "Codex CLI", installed: false },
      ],
    },
    plugin_list: snapshot.plugins,
    usage_snapshot: {
      providers: [
        {
          id: "anthropic",
          windows: ["5h window: 38% used, resets 16:20", "Weekly: 61% used"],
        },
      ],
      sessions: [
        { id: sessions[0].id, tokens: 184320, context_pct: 47 },
        { id: sessions[1].id, tokens: 2140 },
        { id: sessions[2].id, tokens: 902115, context_pct: 92 },
      ],
      projects: [
        { id: projects[0].id, tokens: 1088547 },
        { id: projects[1].id, tokens: 12004 },
      ],
      fetched_at: ago(0),
    },
    project_tree: {
      branch: "main",
      entries: [
        { name: "crates", kind: "dir", git: "M" },
        { name: "apps", kind: "dir", git: "M" },
        { name: "docs", kind: "dir" },
        { name: "designs", kind: "dir" },
        { name: "AGENTS.md", kind: "file" },
        { name: "justfile", kind: "file", git: "M" },
        { name: "README.md", kind: "file" },
        { name: "scratch.log", kind: "file", git: "?" },
      ],
    },
    project_read_file: {
      content:
        "# Willie\n\nA personal control plane for AI coding agents on Windows.\n",
      truncated: false,
    },
    session_open: { session: sessions[0] },
    session_resume: { session: sessions[0] },
    session_terminal_open: null,
  };

  const pluginResults = {
    "profile.list": [
      { name: "default", fragments_active: ["CLAUDE.md", "settings.json"] },
      { name: "strict-review", fragments_active: ["CLAUDE.md"] },
    ],
    "profile.check": {
      changes: [
        {
          path: ".claude/settings.json",
          kind: "merge",
          after: '{\n  "model": "opus"\n}',
        },
        { path: "CLAUDE.md", kind: "overwrite", after: "# Project rules\n" },
      ],
    },
    "profile.read_fragment": { content: "# Project rules\n\nBe brief.\n" },
  };

  function emitTerminal(id) {
    const handlers = listeners.get("session://output") || [];
    const text =
      "\u001b[1;36m willie \u001b[0m ~/projects/willie\r\n" +
      "$ just check\r\n" +
      "\u001b[32m   Checking\u001b[0m willie-core v0.1.0\r\n" +
      "\u001b[32m   Checking\u001b[0m willie-proto v0.1.0\r\n" +
      "\u001b[32m    Finished\u001b[0m `dev` profile in 12.44s\r\n" +
      "\u001b[32mOK\u001b[0m fmt-check  \u001b[32mOK\u001b[0m lint  \u001b[32mOK\u001b[0m test\r\n" +
      "$ ";
    const chunk = Array.from(new TextEncoder().encode(text));

    for (const h of handlers) {
      const cb = cbs.get(h);
      if (cb) cb({ event: "session://output", id: h, payload: { id, chunk } });
    }
  }

  window.__MOCK_CALLS__ = [];

  /* The event plugin unregisters through its own global on unlisten;
   * without it every unmount throws and drowns the real errors. */
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
    unregisterListener(event, id) {
      const list = listeners.get(event) || [];
      listeners.set(
        event,
        list.filter((h) => h !== id),
      );
      cbs.delete(id);
    },
  };

  window.__TAURI_INTERNALS__ = {
    metadata: {
      currentWindow: { label: "main" },
      currentWebview: { windowLabel: "main", label: "main" },
    },
    transformCallback(cb) {
      const id = nextCb++;
      cbs.set(id, cb);

      return id;
    },
    unregisterCallback(id) {
      cbs.delete(id);
    },
    convertFileSrc: (p) => p,
    invoke(cmd, args) {
      window.__MOCK_CALLS__.push(cmd);

      if (cmd === "plugin:event|listen") {
        const list = listeners.get(args.event) || [];
        list.push(args.handler);
        listeners.set(args.event, list);

        return Promise.resolve(args.handler);
      }

      if (cmd === "plugin:dialog|open") {
        return Promise.resolve("D:\\Projects\\marketing-site");
      }

      if (cmd === "plugin_call") {
        return Promise.resolve(pluginResults[args.method] ?? {});
      }

      if (cmd === "session_terminal_open") {
        setTimeout(() => emitTerminal(args.id), 120);

        return Promise.resolve(null);
      }

      if (cmd in results) return Promise.resolve(results[cmd]);

      return Promise.resolve(null);
    },
  };
})();
