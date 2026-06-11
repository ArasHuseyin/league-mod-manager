import { invoke } from "@tauri-apps/api/core";
import type {
  ApplyReport,
  LeagueInstallation,
  LibraryItem,
  PatchReport,
  Profile,
} from "./types";

const demoLibrary: LibraryItem[] = [
  {
    manifest: {
      schemaVersion: 1,
      id: "1f2f0a77-9648-4f7b-ae09-fd9c76995a12",
      name: "Aatrox Crimson VFX",
      version: "0.1.0",
      author: "Workshop",
      description: "Sample mod package for UI development.",
      tags: ["champion", "vfx"],
      previewImage: null,
      assets: [
        {
          source: "assets/aatrox/vfx.bin",
          target: "data/characters/aatrox/skins/skin01/vfx.bin",
          wad: "Characters/Aatrox.wad.client",
          layer: "vfx",
        },
      ],
    },
    packagePath: "samples/aatrox-crimson-vfx",
    importedAt: "1970-01-01T00:00:00Z",
  },
  {
    manifest: {
      schemaVersion: 1,
      id: "fb657393-e6a9-466b-8719-0bfe8223b331",
      name: "SR Minimal HUD",
      version: "0.1.0",
      author: "Workshop",
      description: "Sample interface package for UI development.",
      tags: ["ui", "hud"],
      previewImage: null,
      assets: [
        {
          source: "assets/hud/layout.bin",
          target: "data/menu/hud/layout.bin",
          wad: "DATA/Menu.wad.client",
          layer: "interface",
        },
      ],
    },
    packagePath: "samples/sr-minimal-hud",
    importedAt: "1970-01-01T00:00:00Z",
  },
];

const demoProfiles: Profile[] = [
  {
    id: "6380b3e3-9990-41ef-8969-1df845063f25",
    name: "Ranked safe",
    enabledMods: ["1f2f0a77-9648-4f7b-ae09-fd9c76995a12"],
    modOrder: ["1f2f0a77-9648-4f7b-ae09-fd9c76995a12"],
  },
  {
    id: "de9929c5-cc3f-4644-b2b5-e6f4c15f21a0",
    name: "Workshop testing",
    enabledMods: [],
    modOrder: [],
  },
];

async function call<T>(command: string, args?: Record<string, unknown>, fallback?: T): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (error) {
    if (fallback !== undefined) {
      return fallback;
    }
    throw error;
  }
}

export const api = {
  getLibrary: () => call<LibraryItem[]>("get_library", undefined, demoLibrary),
  getProfiles: () => call<Profile[]>("get_profiles", undefined, demoProfiles),
  detectLeague: () => call<LeagueInstallation[]>("detect_league", undefined, []),
  createProfile: (name: string) => call<Profile>("create_profile", { name }),
  importMod: (path: string) => call<LibraryItem>("import_mod", { path }),
  setProfileModEnabled: (profileId: string, modId: string, enabled: boolean) =>
    call<Profile>("set_profile_mod_enabled", { profileId, modId, enabled }),
  planPatch: (profileId: string, leagueRoot: string) =>
    call<PatchReport>("plan_patch", { profileId, leagueRoot }),
  applyPatch: (profileId: string, leagueRoot: string) =>
    call<ApplyReport>("apply_patch", { profileId, leagueRoot }),
  clearMods: () => call<string>("clear_mods"),
  checkElevation: () => call<boolean>("check_elevation", undefined, true),
  reorderProfileMod: (profileId: string, modId: string, up: boolean) =>
    call<Profile>("reorder_profile_mod", { profileId, modId, up }),
  selectDirectory: () => call<string | null>("select_directory", undefined, null),
  selectFile: () => call<string | null>("select_file", undefined, null),
};
