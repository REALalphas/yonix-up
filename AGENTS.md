# Yonix

Yonix is a NixOS administration companion that unifies configuration checks, builds, closure comparisons, activation, rollback, staged cleanup, and multi-host management across desktop, CLI, and browser interfaces powered by a single shared Rust service.

## Architecture

- **Daemon & CLI (`src/`)**: Written in Rust (edition 2024, axum, tokio, clap). Runs a local HTTP daemon (default: `127.0.0.1:9847`) that serializes Nix operations, serves the web dashboard, manages authentication tokens, and handles single-hop peer routing.
- **Native GUI (`src/gui.rs`)**: Built with GTK 4 and libadwaita (feature `gui`, application ID `com.alphas.yonix`). Connects to the local daemon and renders system status, action triggers, and real-time logs.
- **Browser Panel (`frontend/` → `web/`)**: React 19 + TypeScript single-page application styled using vendored Alpha UI (`@alpha/core`, `@alpha/react`). Bundled assets are compiled into `web/` and embedded directly into the binary at build time.
- **Nix Module & Packaging (`nix/`, `flake.nix`)**: Exposes `programs.yonix` for NixOS configurations, provides reproducible packages (`yonix`, `yonix-headless`), and includes comprehensive VM tests (`nix/vm-test.nix`, `nix/peers-vm-test.nix`, `nix/btrfs-vm-test.nix`).
- **Translations (`locales/`)**: 13 interface languages embedded into both desktop and web binaries, tested for complete key parity.

## Subagents

- Subagents are applicable to separate technical subtasks: local changes, research into a specific question, and independent verification. Use them when a subtask can be clearly separated from the main work.
- Model priority: **6.2 Sol (Low) > Gemini 3.8 Flash (High) > Luna (Max)**. Select the first available model in that order; Low, High, and Max are the reasoning effort settings for 6.2 Sol, Gemini 3.8 Flash, and Luna respectively. Explicit user settings take precedence.
- Subagents **MUST receive small, precise tasks**: one bounded objective, necessary context, specific files or scope, constraints, expected output, and verification criteria. Do not delegate the entire user request or all project work. The primary agent performs a substantial part of the work and owns decomposition, decisions, integration, and final verification. Assign file ownership before parallel edits.
- If a work result is factually verifiable, **launch an independent counter-agent** with a narrow critical review task. Provide requirements, the result or diff, and acceptance criteria. The counter-agent looks for errors and counterexamples, runs relevant available checks, and supplies evidence: a file and line, reproduction, or check output. It explicitly reports unavailable checks and does not present preferences or speculation as factual defects.
- Validate the counter-agent's findings. When factual problems are confirmed, fix them and **launch a new independent agent** to re-check the corrected result, providing the original task, findings, fixes, and remaining objections. Repeat until confirmed problems are resolved; if verification is blocked, report the limitation and do not claim full verification. The primary agent handles small fixes directly; a separate implementer is only needed for a bounded subtask.
- Counter-agents receive equally small, precise tasks and follow the same model priority. Do not ask subagents to spawn further agents: the primary agent coordinates implementers and fresh independent reviews. Delegation does not expand permissions to modify or deploy; these rules apply to active technical work and do not authorize reviving archived projects.

## Frontend & Alpha UI

- Use official Alpha UI releases and public `@alpha/core` and `@alpha/react` APIs. Do not modify or patch vendored archives in `vendor/alpha-ui/`, and do not override component internals with application CSS.
- The production app is an immutable Nix package. Frontend dependencies are installed and rebuilt at package-build time; updating the UI produces a complete new package.
- Never download, replace, or inject production assets at daemon startup.
- After making changes in `frontend/`, rebuild the tracked production assets with `npm run build`, and verify with `npm test`.

## Verification & Architecture

- Maintain translation completeness: all 13 locale files in `locales/*.json` must retain identical key sets.
- Operations that change the system require explicit confirmation and authorization. Tests involving system activation or filesystem maintenance belong in disposable VMs.
- Before completing tasks, run the verification suite:
  - `nix develop --command cargo fmt --check`
  - `nix develop --command cargo clippy --locked --all-targets -- -D warnings`
  - `nix develop --command cargo test --locked`
  - `npm test`
  - `nix build .#default --no-link`
