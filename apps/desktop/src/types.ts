export type ModAsset = {
  source: string;
  target: string;
  wad: string;
  layer?: string | null;
  sha256?: string | null;
};

export type ModManifest = {
  schemaVersion: number;
  id: string;
  name: string;
  version: string;
  author: string;
  description: string;
  tags: string[];
  previewImage?: string | null;
  assets: ModAsset[];
};

export type LibraryItem = {
  manifest: ModManifest;
  packagePath: string;
  importedAt: string;
};

export type Profile = {
  id: string;
  name: string;
  enabledMods: string[];
  modOrder: string[];
};

export type PatchConflict = {
  wad: string;
  target: string;
  modIds: string[];
};

export type PatchOperation = {
  modId: string;
  modName: string;
  source: string;
  target: string;
  wad: string;
};

export type PatchReport = {
  dryRun: boolean;
  startedAt: string;
  finishedAt: string;
  plan: {
    profileId: string;
    operations: PatchOperation[];
    conflicts: PatchConflict[];
    warnings: string[];
  };
  status: "ready" | "blocked" | "applied";
  messages: string[];
};

export type LeagueInstallation = {
  root: string;
  gameExecutable: string;
  source: string;
};

export type ApplyReport = {
  status: "applied" | "blocked";
  stagingDir: string;
  stagedFiles: string[];
  redirectionCount: number;
  injectorStarted: boolean;
  processName: string;
  messages: string[];
};
