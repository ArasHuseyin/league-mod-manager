# Claude.md

## Context

You are working on a League mod manager inspired by LTK Manager and cslol-manager. The project is a greenfield Tauri 2 + React + Rust monorepo.

The current goal is to make the application useful as a complete manager over time:

- Mod library
- Profiles
- Import and validation
- Policy warnings
- Patch dry-runs
- Workshop/editor
- Jobs, logs, settings, diagnostics
- Future WAD patching behind a tested Rust interface

## Architecture Expectations

- Rust owns domain logic.
- React owns presentation and interaction.
- Tauri commands are thin bridges.
- The patcher must remain usable from both desktop and CLI.
- Keep manifests, profiles, and patch plans serializable with `serde`.

## Safety Boundaries

Do not implement anti-cheat bypasses or stealth/evasion behavior.

The first patcher behavior should stay dry-run and deterministic. If real file mutation is added, require explicit user action, fixture coverage, backup/restore behavior, and clear failure reporting.

Policy behavior is currently warning-based. Keep warnings visible and structured so stricter blocking can be added later without changing all call sites.

## Preferred Style

- Keep code direct and explicit.
- Avoid unnecessary abstractions.
- Add comments only where they clarify non-obvious patching or format behavior.
- Preserve the separation between `manager-core`, `manager-patcher`, `manager-cli`, and `apps/desktop`.

## Useful Commands

```powershell
cargo test --workspace
pnpm install
pnpm --filter @league-mod-manager/desktop test
pnpm --filter @league-mod-manager/desktop build
pnpm --filter @league-mod-manager/desktop tauri:dev
```

## Current Defaults

- Platform: Windows first.
- Stack: Tauri 2, React, TypeScript, Rust.
- Engine: own Rust engine, not a direct LeagueToolkit crate integration.
- Patch behavior: dry-run first, real WAD writing later.
- Compliance: warn users about risky mods instead of blocking by default.
