import React, { useEffect } from "react";
import { createRoot } from "react-dom/client";
import { listen } from "@tauri-apps/api/event";
import {
  AlertTriangle,
  Boxes,
  FileArchive,
  FolderSearch,
  Hammer,
  Library,
  ListChecks,
  Play,
  ScrollText,
  Settings,
  ShieldAlert,
  UserRoundCog,
} from "lucide-react";
import { useAppStore } from "./store";
import "./styles.css";

const navItems = [
  { id: "library", label: "Library", icon: Library },
  { id: "profiles", label: "Profiles", icon: UserRoundCog },
  { id: "workshop", label: "Workshop", icon: Hammer },
  { id: "jobs", label: "Jobs", icon: ListChecks },
  { id: "settings", label: "Settings", icon: Settings },
  { id: "logs", label: "Logs", icon: ScrollText },
] as const;

function App() {
  const {
    activeTab,
    error,
    loading,
    load,
    setTab,
    runDryPatch,
    library,
    profiles,
    selectedProfileId,
    importMod,
  } = useAppStore();

  useEffect(() => {
    void load();
  }, [load]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    async function setupDragDrop() {
      try {
        unlisten = await listen<string[]>("tauri://drag-drop", (event) => {
          const files = event.payload;
          if (files && files.length > 0) {
            for (const file of files) {
              void importMod(file);
            }
          }
        });
      } catch (err) {
        console.warn("Drag-and-drop listener could not be registered:", err);
      }
    }
    void setupDragDrop();
    return () => {
      if (unlisten) {
        unlisten();
      }
    };
  }, [importMod]);

  const activeProfile = profiles.find((profile) => profile.id === selectedProfileId);

  return (
    <div className="app-shell">
      <aside className="sidebar">
        <div className="brand">
          <Boxes size={28} />
          <div>
            <strong>League Mod Manager</strong>
            <span>Local workshop build</span>
          </div>
        </div>
        <nav>
          {navItems.map((item) => {
            const Icon = item.icon;
            return (
              <button
                key={item.id}
                className={activeTab === item.id ? "active" : ""}
                onClick={() => setTab(item.id)}
                title={item.label}
              >
                <Icon size={18} />
                <span>{item.label}</span>
              </button>
            );
          })}
        </nav>
        <div className="sidebar-footer">
          <ShieldAlert size={18} />
          <span>Warnings only policy</span>
        </div>
      </aside>

      <main className="workspace">
        <header className="topbar">
          <div>
            <h1>{navItems.find((item) => item.id === activeTab)?.label}</h1>
            <p>
              {library.length} mods · {profiles.length} profiles
              {activeProfile ? ` · Active: ${activeProfile.name}` : ""}
            </p>
          </div>
          <button className="primary" onClick={() => void runDryPatch()} disabled={loading}>
            {loading ? <span className="spinner" /> : <Play size={17} />}
            {loading ? "Patching..." : "Dry patch"}
          </button>
        </header>

        {error ? (
          <div className="banner">
            <AlertTriangle size={18} />
            <span>{error}</span>
          </div>
        ) : null}

        {activeTab === "library" && <LibraryView />}
        {activeTab === "profiles" && <ProfilesView />}
        {activeTab === "workshop" && <WorkshopView />}
        {activeTab === "jobs" && <JobsView />}
        {activeTab === "settings" && <SettingsView />}
        {activeTab === "logs" && <LogsView />}
      </main>
    </div>
  );
}

function LibraryView() {
  const { library, importMod, selectAndImportMod, selectAndImportModDir, loading } = useAppStore();
  const [path, setPath] = React.useState("");
  return (
    <>
      <section className="panel import-panel">
        <h2>Import Mod</h2>
        <div className="form-row">
          <input
            value={path}
            onChange={(event) => setPath(event.target.value)}
            placeholder="Path to a mod folder containing manifest.json"
          />
          <button
            onClick={() => {
              void importMod(path);
              setPath("");
            }}
            disabled={loading || !path.trim()}
          >
            {loading ? <span className="spinner" /> : <FileArchive size={17} />}
            Import
          </button>
          <button
            className="secondary"
            onClick={() => void selectAndImportMod()}
            disabled={loading}
            title="Import a .fantome / .zip archive"
          >
            {loading ? <span className="spinner" /> : <FileArchive size={17} />}
            File...
          </button>
          <button
            className="secondary"
            onClick={() => void selectAndImportModDir()}
            disabled={loading}
            title="Import an unpacked mod folder containing manifest.json"
          >
            {loading ? <span className="spinner" /> : <FolderSearch size={17} />}
            Folder...
          </button>
        </div>
      </section>

      <section className="content-grid">
        {library.map((item) => (
          <article className="mod-card" key={item.manifest.id}>
            <div className="preview">
              <FileArchive size={28} />
            </div>
            <div className="mod-body">
              <div className="row">
                <h2>{item.manifest.name}</h2>
                <span className="version">v{item.manifest.version}</span>
              </div>
              <p>{item.manifest.description}</p>
              <div className="tags">
                {item.manifest.tags.map((tag) => (
                  <span key={tag}>{tag}</span>
                ))}
              </div>
              <dl>
                <div>
                  <dt>Assets</dt>
                  <dd>{item.manifest.assets.length}</dd>
                </div>
                <div>
                  <dt>Author</dt>
                  <dd>{item.manifest.author}</dd>
                </div>
                <div>
                  <dt>Package</dt>
                  <dd>{item.packagePath}</dd>
                </div>
              </dl>
            </div>
          </article>
        ))}
      </section>
    </>
  );
}

function ProfilesView() {
  const {
    profiles,
    library,
    selectedProfileId,
    setSelectedProfile,
    setProfileModEnabled,
    createProfile,
  } = useAppStore();
  const [name, setName] = React.useState("");
  const activeProfile = profiles.find((profile) => profile.id === selectedProfileId);
  return (
    <section className="split">
      <div className="panel">
        <h2>Profiles</h2>
        <div className="form-row compact">
          <input
            value={name}
            onChange={(event) => setName(event.target.value)}
            placeholder="New profile name"
          />
          <button
            onClick={() => {
              void createProfile(name.trim());
              setName("");
            }}
            disabled={!name.trim()}
          >
            Create
          </button>
        </div>
        <div className="stack">
          {profiles.map((profile) => (
            <button
              className={`list-row ${profile.id === selectedProfileId ? "selected" : ""}`}
              key={profile.id}
              onClick={() => setSelectedProfile(profile.id)}
            >
              <div>
                <strong>{profile.name}</strong>
                <span>{profile.enabledMods.length} enabled mods</span>
              </div>
              <span>{profile.modOrder.length} ordered</span>
            </button>
          ))}
        </div>
      </div>
      <div className="panel">
        <h2>Enabled Mods</h2>
        <div className="stack">
          {library.map((item) => (
            <div className="list-row" key={item.manifest.id}>
              <div>
                <strong>{item.manifest.name}</strong>
                <span>{item.manifest.assets[0]?.wad ?? "No WAD target"}</span>
              </div>
              <input
                type="checkbox"
                checked={activeProfile?.enabledMods.includes(item.manifest.id) ?? false}
                onChange={(event) =>
                  void setProfileModEnabled(item.manifest.id, event.currentTarget.checked)
                }
              />
            </div>
          ))}
        </div>
      </div>
    </section>
  );
}

function WorkshopView() {
  return (
    <section className="split">
      <div className="panel">
        <h2>Package Builder</h2>
        <div className="drop-zone">
          <FolderSearch size={32} />
          <span>Drop a raw mod folder or manifest directory</span>
        </div>
      </div>
      <div className="panel">
        <h2>Validation</h2>
        <div className="checklist">
          <span>Manifest schema</span>
          <span>Asset target map</span>
          <span>Policy warnings</span>
          <span>Package export</span>
        </div>
      </div>
    </section>
  );
}

function JobsView() {
  const { lastPatchReport } = useAppStore();
  return (
    <section className="panel">
      <h2>Patch Job</h2>
      {lastPatchReport ? (
        <div className="job-report">
          <div className="metrics">
            <div>
              <strong>{lastPatchReport.status}</strong>
              <span>Status</span>
            </div>
            <div>
              <strong>{lastPatchReport.plan.operations.length}</strong>
              <span>Operations</span>
            </div>
            <div>
              <strong>{lastPatchReport.plan.conflicts.length}</strong>
              <span>Conflicts</span>
            </div>
          </div>
          {lastPatchReport.messages.map((message) => (
            <p key={message}>{message}</p>
          ))}
          {lastPatchReport.plan.conflicts.length > 0 ? (
            <div className="stack">
              <h2>Conflicts</h2>
              {lastPatchReport.plan.conflicts.map((conflict) => (
                <div className="list-row" key={`${conflict.wad}-${conflict.target}`}>
                  <div>
                    <strong>{conflict.target}</strong>
                    <span>{conflict.wad}</span>
                  </div>
                  <span>{conflict.modIds.length} mods</span>
                </div>
              ))}
            </div>
          ) : null}
          {lastPatchReport.plan.operations.length > 0 ? (
            <div className="stack">
              <h2>Operations</h2>
              {lastPatchReport.plan.operations.map((operation) => (
                <div className="list-row" key={`${operation.modId}-${operation.target}`}>
                  <div>
                    <strong>{operation.modName}</strong>
                    <span>{operation.target}</span>
                  </div>
                  <span>{operation.wad}</span>
                </div>
              ))}
            </div>
          ) : null}
        </div>
      ) : (
        <p>No patch job has been run in this session.</p>
      )}
    </section>
  );
}

function SettingsView() {
  const { leagueRoot, setLeagueRoot, selectAndSetLeagueRoot, installations, detectLeague, loading } = useAppStore();
  return (
    <section className="panel">
      <h2>League Installation</h2>
      <div className="form-row">
        <input
          value={leagueRoot}
          onChange={(event) => setLeagueRoot(event.target.value)}
          placeholder="C:\\Riot Games\\League of Legends"
        />
        <button onClick={() => void selectAndSetLeagueRoot()} disabled={loading}>
          {loading ? <span className="spinner" /> : <FolderSearch size={17} />}
          Browse...
        </button>
        <button className="secondary" onClick={() => void detectLeague()} disabled={loading}>
          {loading ? <span className="spinner" style={{ marginRight: 6 }} /> : null}
          Detect
        </button>
      </div>
      <div className="stack">
        {installations.map((installation) => (
          <div className="list-row" key={installation.root}>
            <div>
              <strong>{installation.root}</strong>
              <span>{installation.source}</span>
            </div>
            <span>found</span>
          </div>
        ))}
      </div>
    </section>
  );
}

function LogsView() {
  const { logLines } = useAppStore();
  return (
    <section className="panel logs">
      {logLines.map((line, index) => (
        <code key={`${line}-${index}`}>{line}</code>
      ))}
    </section>
  );
}

createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
