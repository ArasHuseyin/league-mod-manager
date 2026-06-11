# League Mod Manager

A Windows-first desktop app for installing League of Legends skin/asset mods,
built with Tauri + React + Rust. It is inspired by cslol-manager and LTK
Manager and works the same way: it stages patched game archives and injects a
small hook DLL into the running game that redirects the game's file reads to
those staged copies. The live League installation is never modified.

> [!WARNING]
> Modifying League of Legends violates Riot's Terms of Service and can result
> in a ban — this includes the Practice Tool and custom games. Use entirely at
> your own risk. This project is for educational purposes.

## How it works

Applying a profile is a four-step pipeline:

1. **Stage.** For every WAD a mod targets, the patcher locates the *real*
   `.wad.client` archive inside your League installation, applies the mod's
   overrides and additions, and writes a full, game-loadable copy into a
   staging folder. Because the original archive is kept whole, the game still
   finds every untouched asset — only the modded entries change. Raw
   (Fantome `RAW/`) files are copied verbatim.
2. **Generate redirections.** Alongside the staged archives, the patcher writes
   a `redirections.json` mapping each archive's install-relative path to its
   staged replacement.
3. **Inject.** The injector copies the hook DLL next to `redirections.json`,
   then watches for the game process (`League of Legends.exe`) and injects the
   DLL via `LoadLibraryW` as soon as it appears.
4. **Redirect.** Inside the game, the DLL hooks `CreateFileW` (using MinHook)
   and, for any open whose path matches a redirection key, transparently swaps
   in the staged archive instead.

> [!IMPORTANT]
> The hook must go into **`League of Legends.exe`** — the actual game process
> that loads WADs, including in the Practice Tool — **not** `LeagueClient.exe`
> (the launcher). The injector can be armed before or after the game starts.

## Architecture

The repository is a Cargo + pnpm workspace.

| Crate / package | Role |
| --- | --- |
| `crates/manager-core` | Manifests, profiles, validation, policy warnings, League path discovery, patch planning, and persisted state. |
| `crates/manager-patcher` | The WAD codec and patch engine. `league_wad` reads and patches real League WAD v3 archives (`patch_or_add_wad` overrides existing chunks and appends new ones, Gzip-compressing what shrinks). `PatchEngine::stage` produces the patched archives and `redirections.json`, and reports which overrides matched real assets. |
| `crates/manager-injector` | Finds the game process and injects a DLL (`CreateRemoteThread` + `LoadLibraryW`). Usable as a library or a CLI. |
| `crates/manager-hook-dll` | The injected `cdylib`. Reads `redirections.json` from its own directory and hooks `CreateFileW` to redirect WAD opens. |
| `crates/manager-cli` | Developer CLI for validation, import checks, patch dry-runs, staging, and diagnostics. |
| `apps/desktop` | Tauri + React UI: library, profiles, workshop, jobs, settings, and logs, plus the **Apply & inject** flow. |

## Build & run

Prerequisites: Rust (stable), Node.js with `pnpm`, and the
[Tauri prerequisites](https://tauri.app/start/prerequisites/) for your platform.

```powershell
pnpm install
pnpm --filter @league-mod-manager/desktop tauri:dev   # run the desktop app
cargo test --workspace                                 # run all Rust tests
```

The injector and hook DLL are Windows-only and must be built before the
**Apply & inject** button can run:

```powershell
cargo build --release -p manager-injector -p manager-hook-dll
```

In a workspace dev build every crate shares one `target/` directory, so the app
finds `manager-injector.exe` and `manager_hook_dll.dll` next to itself
automatically. In a packaged build, ship them as sidecars in the same folder.
You can also point the app at explicit paths with the
`LEAGUE_MOD_MANAGER_INJECTOR` and `LEAGUE_MOD_MANAGER_HOOK_DLL` environment
variables.

## Desktop workflow

1. **Settings** — set or auto-detect your League installation path.
2. **Library** — import mods (folders with a `manifest.json`, or legacy
   `.fantome` / `.modpkg` archives; drag-and-drop is supported).
3. **Profiles** — create a profile, enable the mods you want, and set their
   **load order**. When two enabled mods change the same asset, the one later in
   the order wins (no hard conflict); use the up/down arrows to choose.
4. **Dry patch** — preview the patch plan and surface overlap warnings without
   writing anything.
5. **Apply & inject** — stage the patched WADs, arm the injector, then launch
   League (or the Practice Tool). Results appear under **Jobs**.
6. **Stop & clear** — disarm the injector and clear staged output. A hook already
   loaded into a running game keeps its redirects until you restart the game.

> [!IMPORTANT]
> Run the app **as Administrator** — injecting into the game requires it, and the
> app warns when it is not elevated.

The **Jobs** view reports how many overrides actually *matched* an existing game
asset versus how many were *appended as new entries*. A non-zero "added" count
usually means a mod declared wrong target paths and that override will have no
visible effect — the first thing to check when a mod "does nothing". Once the
game starts, the injector reports back whether injection actually succeeded.

## CLI

```bash
# Validate a mod package
cargo run -p manager-cli -- validate path/to/mod

# Inspect validation + policy for an import
cargo run -p manager-cli -- import path/to/mod

# Dry-run a patch plan against an installation
cargo run -p manager-cli -- patch --manifest path/to/manifest.json --league-root "C:\\Riot Games\\League of Legends"

# Stage patched WADs + redirections.json into an output directory
cargo run -p manager-cli -- patch --manifest path/to/manifest.json --league-root "C:\\Riot Games\\League of Legends" --out ./staging

# Report detected installations
cargo run -p manager-cli -- doctor
```

## Mod format

A mod is described by a `manifest.json` (`schemaVersion: 1`) listing assets,
each with a `source` (path inside the package), a `target` (in-WAD path), and a
`wad` (the destination archive, e.g. `Characters/Aatrox.wad.client`, or the
`RAW` sentinel for raw files). Legacy `.fantome` archives are normalized into
this model on import.

## Safety boundaries

- The League installation is never modified; all output goes to a staging
  folder and the game is redirected to it at runtime.
- The patcher does not touch, bypass, or tamper with anti-cheat.
- When two enabled mods target the same asset, the overlap is resolved by load
  order (later wins) and reported as a warning, not silently merged.
