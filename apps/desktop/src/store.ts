import { create } from "zustand";
import { api } from "./api";
import type {
  ApplyReport,
  InjectorResult,
  LeagueInstallation,
  LibraryItem,
  PatchReport,
  Profile,
} from "./types";

type AppTab = "library" | "profiles" | "workshop" | "jobs" | "settings" | "logs";

type AppState = {
  activeTab: AppTab;
  library: LibraryItem[];
  profiles: Profile[];
  installations: LeagueInstallation[];
  selectedProfileId?: string;
  leagueRoot: string;
  lastPatchReport?: PatchReport;
  lastApplyReport?: ApplyReport;
  injectorResult?: InjectorResult;
  elevated: boolean;
  logLines: string[];
  loading: boolean;
  applying: boolean;
  error?: string;
  setTab: (tab: AppTab) => void;
  setLeagueRoot: (root: string) => void;
  setSelectedProfile: (profileId: string) => void;
  load: () => Promise<void>;
  detectLeague: () => Promise<void>;
  createProfile: (name: string) => Promise<void>;
  importMod: (path: string) => Promise<void>;
  setProfileModEnabled: (modId: string, enabled: boolean) => Promise<void>;
  reorderMod: (modId: string, up: boolean) => Promise<void>;
  runDryPatch: () => Promise<void>;
  runApplyPatch: () => Promise<void>;
  clearMods: () => Promise<void>;
  setInjectorResult: (result: InjectorResult) => void;
  selectAndSetLeagueRoot: () => Promise<void>;
  selectAndImportMod: () => Promise<void>;
  selectAndImportModDir: () => Promise<void>;
};

export const useAppStore = create<AppState>((set, get) => ({
  activeTab: "library",
  library: [],
  profiles: [],
  installations: [],
  leagueRoot: "",
  elevated: true,
  logLines: ["App shell initialized."],
  loading: false,
  applying: false,
  setTab: (activeTab) => set({ activeTab }),
  setLeagueRoot: (leagueRoot) => set({ leagueRoot }),
  setSelectedProfile: (selectedProfileId) => set({ selectedProfileId }),
  load: async () => {
    set({ loading: true, error: undefined });
    try {
      const [library, profiles, installations, elevated] = await Promise.all([
        api.getLibrary(),
        api.getProfiles(),
        api.detectLeague(),
        api.checkElevation(),
      ]);
      set({
        library,
        profiles,
        installations,
        elevated,
        selectedProfileId: profiles[0]?.id,
        leagueRoot: installations[0]?.root ?? "",
        loading: false,
        logLines: [
          ...get().logLines,
          `Loaded ${library.length} library items and ${profiles.length} profiles.`,
        ],
      });
    } catch (error) {
      set({ loading: false, error: String(error) });
    }
  },
  detectLeague: async () => {
    const installations = await api.detectLeague();
    set({
      installations,
      leagueRoot: installations[0]?.root ?? get().leagueRoot,
      logLines: [...get().logLines, `Detected ${installations.length} League installation(s).`],
    });
  },
  createProfile: async (name) => {
    const profile = await api.createProfile(name).catch(() => ({
      id: crypto.randomUUID(),
      name,
      enabledMods: [],
      modOrder: [],
    }));
    set({
      profiles: [...get().profiles, profile],
      selectedProfileId: profile.id,
      logLines: [...get().logLines, `Created profile "${profile.name}".`],
    });
  },
  importMod: async (path) => {
    set({ loading: true, error: undefined });
    try {
      const item = await api.importMod(path);
      set({
        loading: false,
        library: [...get().library, item],
        logLines: [...get().logLines, `Imported "${item.manifest.name}".`],
      });
    } catch (error) {
      set({ loading: false, error: String(error) });
    }
  },
  setProfileModEnabled: async (modId, enabled) => {
    const { selectedProfileId, profiles } = get();
    if (!selectedProfileId) {
      set({ error: "Select a profile before changing mods." });
      return;
    }

    const fallback = profiles.map((profile) => {
      if (profile.id !== selectedProfileId) {
        return profile;
      }
      const enabledMods = enabled
        ? Array.from(new Set([...profile.enabledMods, modId]))
        : profile.enabledMods.filter((id) => id !== modId);
      const modOrder = profile.modOrder.includes(modId)
        ? profile.modOrder
        : [...profile.modOrder, modId];
      return { ...profile, enabledMods, modOrder };
    });

    try {
      const updated = await api.setProfileModEnabled(selectedProfileId, modId, enabled);
      set({
        profiles: profiles.map((profile) => (profile.id === updated.id ? updated : profile)),
        logLines: [
          ...get().logLines,
          `${enabled ? "Enabled" : "Disabled"} mod ${modId} in active profile.`,
        ],
      });
    } catch {
      set({ profiles: fallback });
    }
  },
  runDryPatch: async () => {
    const { selectedProfileId, leagueRoot } = get();
    if (!selectedProfileId) {
      set({ error: "Select a profile before running a dry patch." });
      return;
    }
    if (!leagueRoot.trim()) {
      set({ error: "Set the League installation path before running a dry patch." });
      return;
    }

    set({ loading: true, error: undefined });
    try {
      const report = await api.planPatch(selectedProfileId, leagueRoot);
      set({
        loading: false,
        lastPatchReport: report,
        activeTab: "jobs",
        logLines: [
          ...get().logLines,
          `Dry patch finished with status ${report.status} and ${report.plan.conflicts.length} conflict(s).`,
        ],
      });
    } catch (error) {
      set({ loading: false, error: String(error) });
    }
  },
  runApplyPatch: async () => {
    const { selectedProfileId, leagueRoot } = get();
    if (!selectedProfileId) {
      set({ error: "Select a profile before applying mods." });
      return;
    }
    if (!leagueRoot.trim()) {
      set({ error: "Set the League installation path before applying mods." });
      return;
    }

    set({ applying: true, error: undefined, injectorResult: undefined });
    try {
      const report = await api.applyPatch(selectedProfileId, leagueRoot);
      set({
        applying: false,
        lastApplyReport: report,
        elevated: report.elevated,
        activeTab: "jobs",
        logLines: [
          ...get().logLines,
          report.injectorStarted
            ? `Applied ${report.stagedFiles.length} file(s) (${report.matchedOverrides} matched, ${report.addedEntries} added) with ${report.redirectionCount} redirection(s); injector watching for ${report.processName}.`
            : `Apply finished with status ${report.status}: ${report.messages.join(" ")}`,
        ],
      });
    } catch (error) {
      set({ applying: false, error: String(error) });
    }
  },
  clearMods: async () => {
    try {
      const message = await api.clearMods();
      set({
        lastApplyReport: undefined,
        injectorResult: undefined,
        logLines: [...get().logLines, message],
      });
    } catch (error) {
      set({ error: String(error) });
    }
  },
  setInjectorResult: (result) =>
    set({
      injectorResult: result,
      logLines: [...get().logLines, result.message],
    }),
  reorderMod: async (modId, up) => {
    const { selectedProfileId, profiles } = get();
    if (!selectedProfileId) {
      set({ error: "Select a profile before reordering mods." });
      return;
    }
    try {
      const updated = await api.reorderProfileMod(selectedProfileId, modId, up);
      set({
        profiles: profiles.map((profile) => (profile.id === updated.id ? updated : profile)),
      });
    } catch (error) {
      set({ error: String(error) });
    }
  },
  selectAndSetLeagueRoot: async () => {
    try {
      const path = await api.selectDirectory();
      if (path) {
        set({ leagueRoot: path });
        set({ logLines: [...get().logLines, `Selected League path: ${path}`] });
      }
    } catch (error) {
      set({ error: String(error) });
    }
  },
  selectAndImportMod: async () => {
    try {
      const path = await api.selectFile();
      if (path) {
        await get().importMod(path);
      }
    } catch (error) {
      set({ error: String(error) });
    }
  },
  selectAndImportModDir: async () => {
    try {
      const path = await api.selectDirectory();
      if (path) {
        await get().importMod(path);
      }
    } catch (error) {
      set({ error: String(error) });
    }
  },
}));
