# League Mod Manager

A Windows-first Tauri + React + Rust desktop app scaffold inspired by LTK Manager and cslol-manager.

The first implementation includes:

- A Rust core crate for manifests, profiles, validation, policy warnings, League path discovery, and patch planning.
- A Rust patcher crate with a dry-run engine and explicit safety boundaries.
- A CLI for validation, import checks, patch dry-runs, and diagnostics.
- A Tauri desktop shell with a React UI for library, profiles, workshop, jobs, settings, and logs.

The patcher does not bypass anti-cheat or ship League-specific binary patching. It produces deterministic patch plans and reports so WAD operations can be added behind the same interface later.

## Development

```powershell
pnpm install
pnpm dev
cargo test --workspace
```
