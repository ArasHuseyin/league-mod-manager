# AGENTS.md

## Project

This is a Windows-first League mod manager built as a Tauri 2 + React + Rust monorepo.

Important areas:

- `crates/manager-core`: data model, manifests, validation, policy, profiles, League discovery, patch planning.
- `crates/manager-patcher`: patch engine facade and dry-run patch reports.
- `crates/manager-cli`: developer CLI.
- `apps/desktop`: React frontend and Tauri backend.

## Working Rules

- Prefer small, scoped changes that preserve the Rust library boundary between core, patcher, CLI, and desktop.
- Keep patching logic out of the React app. The UI should call Tauri commands, and Tauri should delegate to Rust crates.
- Keep policy checks centralized in `manager-core`.
- Do not add anti-cheat bypass logic, stealth behavior, or evasion instructions.
- Treat real WAD writing as high risk. Add fixture tests and dry-run coverage before implementing file mutation.
- Use ASCII by default unless the edited file already uses non-ASCII text.

## Commands

Use these from the repository root:

```powershell
cargo test --workspace
pnpm install
pnpm --filter @league-mod-manager/desktop test
pnpm --filter @league-mod-manager/desktop build
pnpm --filter @league-mod-manager/desktop tauri:dev
```

## Implementation Notes

- `.modpkg` is the primary internal package concept.
- `.fantome` should be implemented as legacy import compatibility and normalized into the same manifest/library model.
- Dry-run patch reports are the default acceptance path until binary patching is implemented.
- The app should warn about paid-skin replicas or gameplay-affecting mods, but current policy is warning-based rather than blocking-based.

## Verification

Before handing off changes, run the most relevant checks:

- Rust-only changes: `cargo test --workspace`
- Frontend-only changes: `pnpm --filter @league-mod-manager/desktop test`
- Tauri/backend integration: `pnpm --filter @league-mod-manager/desktop build` and, when practical, `pnpm --filter @league-mod-manager/desktop tauri:dev`
