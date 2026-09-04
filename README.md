# Forge Workspace

A desktop app for orchestrating coding agents in isolated git worktrees, built on Tauri, React, and TypeScript.

## Prerequisites

- **Node.js** (LTS)
- **Rust** via [rustup](https://rustup.rs) — required for `npm run tauri dev` / `npm run tauri build`, not for the frontend-only dev preview
- **Windows**: the "Desktop development with C++" workload from the MSVC Build Tools, plus the WebView2 runtime (bundled with Windows 11, otherwise [downloadable here](https://developer.microsoft.com/microsoft-edge/webview2/))
- **macOS**: Xcode Command Line Tools (`xcode-select --install`)

## Setup

```bash
npm install
```

Frontend-only preview (no Rust toolchain needed — this is what Phase 1 M1 ships):

```bash
npm run dev
```

Full desktop app, once Rust is installed:

```bash
npm run tauri dev
```

## Status

This repository is **Phase 1 (Foundation)** of a 7-phase roadmap:

1. **Foundation** — app shell, design system, SQLite schema, Rust backend skeleton
2. **Project & Repository Management** — opening/creating projects, git repository registration, file browsing, diffing
3. **Agent Execution Isolation** — git-worktree-isolated workspaces, PTY-backed terminals
4. **Model & Secrets Configuration** — API key storage via OS keychain, model configuration
5. **Agent Orchestration** — the Anthropic tool-loop, agent runs, activity streaming, notifications
6. **Tasks & Collaboration** — task tracking tied to agent runs
7. **Project Brain & Skills** — persistent project knowledge and reusable skills

**Phase 1 Milestone M1 (app shell + design system, no backend) is complete.** The React app, routing, design tokens, Zustand stores, and shadcn-style UI primitives all work today with `npm run dev` — there's no fake data anywhere, only honest empty states with disabled affordances explaining what's missing and when it lands.

**M2 and later require the Rust toolchain.** `src-tauri/` currently holds a compileable-in-intent skeleton (module structure, command signatures, DB models, migrations) that has **not been built or run** — Rust isn't installed on the machine this was scaffolded on. Nothing in `src-tauri/` has been compiled.

## Architecture

Forge Workspace pairs a React/TypeScript frontend with a Rust backend (via Tauri 2) that owns everything native: SQLite persistence (through `rusqlite` + `r2d2` pooling), git operations (shelling out to the system `git` binary, including `git worktree` for isolating each agent's changes from the primary checkout and from each other), PTY-backed terminals (`portable-pty`), and OS keychain-backed secret storage (`keyring`) for provider API keys. Agent execution is a custom Anthropic tool-use loop (no third-party agent framework) that streams tool calls and model output back to the frontend as typed Tauri events, persisting every step to SQLite so agent runs survive app restarts and can be replayed in the activity log. The schema for all of this (projects, repositories, agents, agent runs, tool calls, activity events, workspaces, tasks, model configs, notifications) is already fully designed and lives in `src-tauri/migrations/0001_init.sql`, even though most of it won't be read or written until Phases 2–6 land.

## License

MIT — see [LICENSE](./LICENSE).
