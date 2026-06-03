# Plan: League Mod Manager

## Summary

Build a Windows-first desktop mod manager inspired by LTK Manager and cslol-manager. The project uses Tauri 2, React, TypeScript, and Rust. The Rust side owns manifests, profiles, policy warnings, League installation discovery, patch planning, and the patcher interface. The UI owns library browsing, profiles, workshop workflows, jobs, settings, and logs.

The patcher is implemented as a separate Rust library plus CLI so it can be tested without the desktop app. The first engine produces deterministic dry-run patch plans and conflict reports. Real League WAD writing should be added behind the same interface later.

## Key Interfaces

- `crates/manager-core`: manifests, package validation, policy warnings, profiles, storage paths, League installation discovery, and patch-plan generation.
- `crates/manager-patcher`: patch engine facade, dry-run reports, status model, and future binary patching boundary.
- `crates/manager-cli`: developer CLI for validate, import, patch dry-run, and doctor commands.
- `apps/desktop`: Tauri + React application shell.

Primary package format is `.modpkg`, represented internally by a `manifest.json` with assets, target WADs, preview metadata, tags, and author/version data. `.fantome` support is treated as legacy import compatibility and should normalize into the same internal model.

## Implementation Phases

1. Foundation
   - Scaffold Rust workspace and Tauri/React app.
   - Add Windows-first app shell with navigation for Library, Profiles, Workshop, Jobs, Settings, and Logs.
   - Add League path auto-detection and manual path entry.

2. Library and Profiles
   - Import mods from directories and archives.
   - Validate manifests and show policy warnings.
   - Add profile creation, enabled mod toggles, mod ordering, and conflict display.

3. Patcher Engine
   - Build patch plans from active profiles.
   - Detect duplicate WAD target writes and missing library entries.
   - Keep dry-run behavior as the default until real WAD writing is implemented and tested.

4. Workshop
   - Add project editor for manifest, assets, layers, preview metadata, validation, and export.
   - Support raw folder import and `.modpkg` build output.

5. Product Features
   - Add migration helpers for cslol-/LTK-like folders.
   - Add tray integration, update flow, diagnostics, log viewer, and cache cleanup.

6. Real WAD Writing
   - Implement reading and writing of League `.wad.client` archives (header, chunk
     table, compression, and path hashing) behind the existing `PatchEngine` interface.
   - Build an overlay WAD from a profile's active assets instead of mutating original
     game files, so the source installation stays untouched.
   - Keep output directed at a separate staging/output folder first; writing into or
     mounting against a live League installation is a later, separately evaluated step.
   - Cover the engine with fixture tests for small synthetic WADs (round-trip read/write,
     overlay merge, and conflict handling) before any installation-facing behavior.
   - This phase ships no anti-cheat bypass, injection, or stealth behavior. Live use is
     the user's responsibility under Riot policy, consistent with Policy and Safety below.

## Policy and Safety

The current policy is warning-based, not blocking-based. The app should warn about likely paid-skin replicas, premium cosmetic terms, or gameplay-affecting changes. The architecture keeps policy checks centralized so blocking rules can be added later.

The project must not implement anti-cheat bypass logic, stealth behavior, or instructions intended to evade detection. The app should clearly state that users are responsible for Riot policy compliance.

## Test Plan

- Rust unit tests for manifests, validation, policy assessment, profiles, patch-plan generation, and patcher reports.
- Fixture tests for `.modpkg` and `.fantome` import once archive parsing exists.
- Fixture tests for WAD read/write round-trip and overlay merge on small synthetic archives.
- CLI smoke tests for validate, import, patch dry-run, and doctor.
- Frontend tests for navigation, library rendering, profile selection, import states, policy warnings, and patch job results.
- Manual Windows acceptance test: install dependencies, launch app, detect/select League path, import a sample mod, create a profile, run dry-run patch, inspect conflicts/logs.

## Assumptions

- Windows 10/11 is the primary supported platform.
- Tauri + React + Rust is the chosen stack.
- The patcher is a separate Rust library first, with UI and CLI as clients.
- Existing LeagueToolkit crates are not the implementation base; this project builds its own engine.
- The first implementation is allowed to scaffold incomplete archive/WAD operations as explicit, well-tested boundaries.
