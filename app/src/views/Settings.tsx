import { useState } from "react";
import { open as pickDialog } from "@tauri-apps/plugin-dialog";
import { api, type GraphicsSettings, type Msaa, type RenderMode, type Texture, type ThemeKind } from "../api";
import { Btn, Field, Toggle } from "../components";
import { useApp } from "../store";
import { ChangePasswordDialog, ConfirmDialog, ImportDialog, VaultSetupDialog } from "../dialogs";

const THEMES: Array<{ id: ThemeKind; name: string; bg: string; fg: string }> = [
  { id: "System", name: "System", bg: "#1e1f22", fg: "#f5f5f4" },
  { id: "Light", name: "Light", bg: "#f5f5f4", fg: "#2f6fb5" },
  { id: "Dark", name: "Dark", bg: "#1e1f22", fg: "#5b9be0" },
  { id: "Mocha", name: "Catppuccin Mocha", bg: "#1e1e2e", fg: "#cba6f7" },
  { id: "Macchiato", name: "Macchiato", bg: "#24273a", fg: "#c6a0f6" },
  { id: "Frappe", name: "Frappé", bg: "#303446", fg: "#ca9ee6" },
  { id: "Latte", name: "Latte", bg: "#eff1f5", fg: "#8839ef" },
  { id: "Nord", name: "Nord", bg: "#2e3440", fg: "#88c0d0" },
  { id: "Dracula", name: "Dracula", bg: "#282a36", fg: "#bd93f9" },
  { id: "Gruvbox", name: "Gruvbox", bg: "#282828", fg: "#fe8019" },
  { id: "RosePine", name: "Rosé Pine", bg: "#191724", fg: "#eb6f92" },
  { id: "Lineage", name: "Lineage", bg: "#1b1d1a", fg: "#8fa37a" },
];
const ACCENTS = [0xcba6f7, 0x89b4fa, 0x94e2d5, 0xa6e3a1, 0xf9e2af, 0xfab387, 0xf38ba8, 0xeb6f92, 0xf5c2e7, 0x5b9be0];

function Seg<T extends string>({ value, options, onChange, sm }: { value: T; options: Array<[T, string]>; onChange: (v: T) => void; sm?: boolean }) {
  return (
    <div className={`seg ${sm ? "sm" : ""}`}>
      {options.map(([v, label]) => <button key={v} type="button" className={value === v ? "active" : ""} onClick={() => onChange(v)}>{label}</button>)}
    </div>
  );
}

function PerfRow({ label, hint, children }: { label: string; hint?: string; children: React.ReactNode }) {
  return (
    <div className="perf-row">
      <div className="lbl">{label}</div>
      {hint && <div className="hint">{hint}</div>}
      <div className="ctl">{children}</div>
    </div>
  );
}

function Switch({ checked, onChange, disabled }: { checked: boolean; onChange: (v: boolean) => void; disabled?: boolean }) {
  return <label className="toggle"><input type="checkbox" checked={checked} disabled={disabled} onChange={(e) => onChange(e.target.checked)} /><span className="track" /></label>;
}

/** Max alts / Balanced / Roblox defaults, modelled on RoALT's presets. */
const PRESETS: Record<string, Partial<GraphicsSettings>> = {
  max: { enabled: true, fps_cap: 10, low_graphics: true, lighter_rendering: true, render: { msaa: "X1", texture: "Lowest", mode: "Default", gray_sky: true, no_grass: true, lowest_frm: true, pause_voxelizer: true } },
  balanced: { enabled: true, fps_cap: 30, low_graphics: true, lighter_rendering: true, render: { msaa: "X1", texture: "Lowest", mode: "Default", gray_sky: false, no_grass: true, lowest_frm: true, pause_voxelizer: false } },
  defaults: { enabled: false, fps_cap: 0, low_graphics: false, lighter_rendering: false },
};

export function SettingsView({ onNeedUnlock, onShowUpdate }: { onNeedUnlock: () => void; onShowUpdate: () => void }) {
  const { snap, settings, saveSettings, saveGraphics, refresh, toast, update, checkForUpdate } = useApp();
  const [dlg, setDlg] = useState<null | "setup" | "change" | "removeVault" | "import">(null);
  const [exportCookies, setExportCookies] = useState(false);
  const [checkMsg, setCheckMsg] = useState("");
  const [flagsText, setFlagsText] = useState<string | null>(null);
  const [flagsErr, setFlagsErr] = useState("");
  if (!snap || !settings) return null;
  const v = snap.vault;
  const g = snap.graphics;
  const hex = settings.accent != null ? "#" + (settings.accent & 0xffffff).toString(16).padStart(6, "0") : "";
  const mainName = snap.accounts.find((a) => a.is_main)?.username;

  const exportNow = async () => {
    if (exportCookies && !v.unlocked) { onNeedUnlock(); return; }
    const dir = await pickDialog({ directory: true, title: "Choose where to write AccountData.json" });
    if (!dir || Array.isArray(dir)) return;
    try { toast(await api.exportAccounts(dir, exportCookies), "good"); } catch (e) { toast(String(e), "bad"); }
  };
  const render = (patch: Partial<GraphicsSettings["render"]>) => saveGraphics({ render: { ...g.render, ...patch } });
  const saveFlags = async () => {
    if (flagsText === null) return;
    try {
      const parsed = flagsText.trim() ? JSON.parse(flagsText) : {};
      if (typeof parsed !== "object" || Array.isArray(parsed) || parsed === null) throw new Error("Must be a JSON object of \"Flag\": \"value\" pairs.");
      const flags: Record<string, string> = {};
      for (const [k, val] of Object.entries(parsed)) flags[k] = typeof val === "string" ? val : JSON.stringify(val);
      await saveGraphics({ custom_flags: flags });
      setFlagsText(null); setFlagsErr("");
    } catch (e) { setFlagsErr(String((e as Error).message ?? e)); }
  };

  return (
    <div className="settings">
      <section className="card">
        <div className="card-h">Appearance</div>
        <div className="card-b">
          <div className="theme-grid">
            {THEMES.map((t) => (
              <button key={t.id} type="button" className={`theme-chip ${settings.theme === t.id ? "active" : ""}`} onClick={() => saveSettings({ theme: t.id })}>
                <span className="sw"><i style={{ background: t.bg }} /><i style={{ background: t.fg }} /></span>{t.name}
              </button>
            ))}
          </div>
          <Field label="Accent">
            <div className="accent-row">
              <button type="button" className={`accent-dot ${settings.accent == null ? "active" : ""}`} style={{ background: "conic-gradient(#cba6f7,#89b4fa,#a6e3a1,#f9e2af,#f38ba8,#cba6f7)" }} title="Theme default" onClick={() => saveSettings({ accent: null })} />
              {ACCENTS.map((c) => <button key={c} type="button" className={`accent-dot ${settings.accent === c ? "active" : ""}`} style={{ background: "#" + c.toString(16).padStart(6, "0") }} onClick={() => saveSettings({ accent: c })} />)}
              <input className="input" style={{ width: 110 }} placeholder="#RRGGBB" defaultValue={hex} key={hex}
                onBlur={(e) => { const m = e.target.value.trim().match(/^#?([0-9a-fA-F]{6})$/); if (m) void saveSettings({ accent: parseInt(m[1], 16) }); }} />
            </div>
          </Field>
          <div className="kv">
            <span>Rows</span>
            <Seg sm value={settings.density} options={[["Compact", "Compact"], ["Normal", "Normal"], ["Comfortable", "Comfortable"], ["Large", "Large"]]} onChange={(d) => saveSettings({ density: d })} />
            <span />
            <div className="hint">Compact shows one line per account; the others show the alias and description underneath.</div>
          </div>
          <div className="kv">
            <span>Corners</span>
            <input type="range" min={0} max={16} value={settings.corner_radius} onChange={(e) => saveSettings({ corner_radius: Number(e.target.value) })} />
          </div>
        </div>
      </section>

      <section className="card">
        <div className="card-h">Launching</div>
        <div className="card-b">
          <Toggle checked={settings.multi_instance} onChange={(c) => saveSettings({ multi_instance: c })} label="Run several Roblox clients at once"
            hint={`Holds Roblox's single-instance lock so more than one client can run; never modifies the client. ${snap.multi_instance_held ? "Active." : settings.multi_instance ? "Another program already holds the lock (close Roblox or the other manager)." : ""}`} />
          <div className="kv">
            <span>Seconds between launches</span>
            <input className="input" type="number" min={0} max={120} value={settings.launch_spacing_secs} onChange={(e) => saveSettings({ launch_spacing_secs: Math.max(0, Math.min(120, Number(e.target.value) || 0)) })} />
          </div>
          <Toggle checked={settings.include_main_in_batch} onChange={(c) => saveSettings({ include_main_in_batch: c })} label="Include Main in Select All" />
          <div className="hint">Launches go through <b>{snap.launcher_name}</b>, the program registered for roblox-player links. Open Browser uses <b>{snap.browser_name || "no usable browser"}</b>{snap.browser_name ? " with a separate profile per account" : " (install Brave, Chrome or Edge)"}.</div>
        </div>
      </section>

      <section className="card wide">
        <div className="card-h">Performance — alt clients</div>
        <div className="card-b">
          <div className="hint">
            Lower each alt client's FPS and graphics. The FPS cap and quality go into Roblox's own settings file; rendering flags go into the FastFlag file {snap.launcher_name === "Roblox" ? "next to the client" : `${snap.launcher_name} applies at launch`}. Both are written just before a client starts and put back as soon as it has read them, so your own Roblox keeps its settings.
            {mainName ? <> <b>{mainName}</b> is Main and is never touched.</> : <> Mark your own account as <b>Main</b> so it is never touched.</>}
          </div>
          <div className="row" style={{ gap: 6, flexWrap: "wrap" }}>
            <Btn sm onClick={() => saveGraphics(PRESETS.max!)}>Max alts (10 FPS, lowest)</Btn>
            <Btn sm onClick={() => saveGraphics(PRESETS.balanced!)}>Balanced (30 FPS)</Btn>
            <Btn sm onClick={() => saveGraphics(PRESETS.defaults!)}>Roblox defaults (off)</Btn>
          </div>
          <div>
            <PerfRow label="Apply performance settings to alt clients" hint="Off: nothing is written anywhere.">
              <Switch checked={g.enabled} onChange={(c) => saveGraphics({ enabled: c })} />
            </PerfRow>
            <PerfRow label="FPS cap" hint="Frames per second each alt client draws (5–360). 0 leaves the user's own cap. Override it per account in its details; Main is exempt.">
              <input className="input" type="number" min={0} max={360} value={g.fps_cap} onChange={(e) => saveGraphics({ fps_cap: Math.max(0, Math.min(360, Number(e.target.value) || 0)) })} /><span className="muted">FPS</span>
            </PerfRow>
            <PerfRow label="Lowest graphics" hint="Graphics quality level 1 for alts. Saves most of the GPU and CPU.">
              <Switch checked={g.low_graphics} onChange={(c) => saveGraphics({ low_graphics: c })} />
            </PerfRow>
            <PerfRow label="Lighter rendering" hint="FastFlags that cut detail further. Uses much less GPU memory.">
              <Switch checked={g.lighter_rendering} onChange={(c) => saveGraphics({ lighter_rendering: c })} />
            </PerfRow>
            {g.lighter_rendering && (
              <div className="perf-sub">
                <PerfRow label="Anti-aliasing (MSAA)" hint="Lower is faster, edges look jagged.">
                  <Seg sm value={g.render.msaa} options={[["Default", "Default"], ["X1", "x1"], ["X2", "x2"], ["X4", "x4"]] as Array<[Msaa, string]>} onChange={(msaa) => render({ msaa })} />
                </PerfRow>
                <PerfRow label="Texture quality" hint="Lowest uses the least GPU memory.">
                  <Seg sm value={g.render.texture} options={[["Default", "Default"], ["Lowest", "Lowest"], ["Low", "Low"], ["Medium", "Medium"], ["High", "High"]] as Array<[Texture, string]>} onChange={(texture) => render({ texture })} />
                </PerfRow>
                <PerfRow label="Rendering mode" hint="Change only for artifacts or crashes. Roblox may ignore Vulkan and OpenGL.">
                  <Seg sm value={g.render.mode} options={[["Default", "Roblox default"], ["D3D11", "Direct3D 11"], ["Vulkan", "Vulkan"], ["OpenGl", "OpenGL"]] as Array<[RenderMode, string]>} onChange={(mode) => render({ mode })} />
                </PerfRow>
                <PerfRow label="Gray sky" hint="Replaces the skybox with flat gray."><Switch checked={g.render.gray_sky} onChange={(gray_sky) => render({ gray_sky })} /></PerfRow>
                <PerfRow label="No grass" hint="Removes rendered grass."><Switch checked={g.render.no_grass} onChange={(no_grass) => render({ no_grass })} /></PerfRow>
                <PerfRow label="Lowest render quality" hint="Forces Roblox's lowest internal quality level."><Switch checked={g.render.lowest_frm} onChange={(lowest_frm) => render({ lowest_frm })} /></PerfRow>
                <PerfRow label="Pause lighting updates" hint="Stops voxel lighting updates; lighting can look wrong."><Switch checked={g.render.pause_voxelizer} onChange={(pause_voxelizer) => render({ pause_voxelizer })} /></PerfRow>
              </div>
            )}
            <PerfRow label="Custom FastFlags" hint="Your own flags, written for every alt client after the ones above (so they win). Applied even for Full quality accounts.">
              {flagsText === null
                ? <Btn sm onClick={() => setFlagsText(JSON.stringify(g.custom_flags, null, 2))}>Edit… ({Object.keys(g.custom_flags).length})</Btn>
                : <Btn sm kind="primary" onClick={saveFlags}>Save</Btn>}
            </PerfRow>
            {flagsText !== null && (
              <div>
                <textarea className="textarea mono" rows={8} style={{ width: "100%" }} value={flagsText} onChange={(e) => setFlagsText(e.target.value)} placeholder={'{\n  "DFIntTaskSchedulerTargetFps": "60"\n}'} />
                {flagsErr && <div className="bad">{flagsErr}</div>}
                <div className="row" style={{ marginTop: 6 }}><Btn sm kind="ghost" onClick={() => { setFlagsText(null); setFlagsErr(""); }}>Cancel</Btn></div>
              </div>
            )}
          </div>
        </div>
      </section>

      <section className="card">
        <div className="card-h">Security — cookie vault</div>
        <div className="card-b">
          <div className="hint">
            {!v.configured ? "Not set up. Stored cookies let RAM launch, check and open the browser as each account without signing in again." :
              v.mode === "dpapi" ? `Windows account (DPAPI), ${v.unlocked ? "unlocked" : "locked"} · ${v.cookies} cookie${v.cookies === 1 ? "" : "s"}.` :
              `Master password (Argon2id + AES‑256‑GCM), ${v.unlocked ? "unlocked" : "locked"} · ${v.cookies} cookie${v.cookies === 1 ? "" : "s"}.`}
          </div>
          <div className="row" style={{ flexWrap: "wrap" }}>
            {!v.configured && <Btn kind="primary" onClick={() => setDlg("setup")}>Set up vault…</Btn>}
            {v.configured && !v.unlocked && <Btn kind="primary" onClick={onNeedUnlock}>Unlock</Btn>}
            {v.configured && v.unlocked && <Btn onClick={async () => { await api.vaultLock(); await refresh(); }}>Lock</Btn>}
            {v.configured && v.mode === "password" && <Btn onClick={() => setDlg("change")}>Change password…</Btn>}
            {v.configured && <Btn kind="danger" onClick={() => setDlg("removeVault")}>Remove vault…</Btn>}
          </div>
          {v.configured && (
            <>
              <Toggle checked={v.require_hello} onChange={(c) => api.vaultOptions(v.auto_lock_secs, c).then(refresh)} label="Require Windows Hello to reveal or copy a cookie" hint={v.hello_available ? "Windows Hello is available on this PC." : "Windows Hello is not set up on this PC; the master password is asked instead."} />
              {v.mode === "password" && (
                <div className="kv">
                  <span>Lock after idle (minutes)</span>
                  <input className="input" type="number" min={0} max={1440} defaultValue={Math.round(v.auto_lock_secs / 60)} key={v.auto_lock_secs}
                    onBlur={(e) => api.vaultOptions(Math.max(0, Number(e.target.value) || 0) * 60, v.require_hello).then(refresh)} />
                </div>
              )}
            </>
          )}
        </div>
      </section>

      <section className="card">
        <div className="card-h">Data</div>
        <div className="card-b">
          <div className="row"><Btn onClick={() => setDlg("import")}>Import from AccountData.json…</Btn><Btn onClick={exportNow}>Export AccountData.json…</Btn></div>
          <Toggle checked={exportCookies} onChange={setExportCookies} label="Include cookies in the export (DPAPI-encrypted, this PC only)" />
          <div className="row"><Btn kind="ghost" onClick={() => api.openDataFolder().catch((e) => toast(String(e), "bad"))}>Open data folder</Btn><span className="hint ellipsis">{snap.data_dir}</span></div>
        </div>
      </section>

      <section className="card">
        <div className="card-h">Updates and about</div>
        <div className="card-b">
          <div>RAM {snap.version}{update && update.state !== "dismissed" ? <> — <a href="#" onClick={(e) => { e.preventDefault(); onShowUpdate(); }}>{update.version} is available</a></> : null}</div>
          <Toggle checked={settings.check_updates} onChange={(c) => saveSettings({ check_updates: c })} label="Check for RAM updates" hint="Asks GitHub Releases after start and every 6 hours. A new version is offered in the sidebar, never installed without your click; its installer must carry RAM's release signature." />
          <div className="row"><Btn sm onClick={async () => { setCheckMsg("Checking…"); setCheckMsg(await checkForUpdate(true)); }}>Check now</Btn><span className="hint">{checkMsg}</span></div>
          <div className="row"><Btn sm kind="ghost" onClick={() => api.openUrl(`https://github.com/${snap.github_repo}/releases`).catch(() => {})}>Release notes on GitHub</Btn></div>
          <div className="hint">A rewrite of <b>Roblox Account Manager</b> by ic3w0lf22 (GPL‑3.0). Cookie check and performance model from RoALT (MIT); embedded-browser ideas from alyae (GPL‑3.0); layout cues from niccs' RAM v4 (GPL‑3.0). Not affiliated with Roblox Corporation. See NOTICE.md.</div>
        </div>
      </section>

      {dlg === "setup" && <VaultSetupDialog onClose={() => { setDlg(null); void refresh(); }} />}
      {dlg === "change" && <ChangePasswordDialog onClose={() => setDlg(null)} />}
      {dlg === "import" && <ImportDialog onClose={() => setDlg(null)} />}
      {dlg === "removeVault" && <ConfirmDialog danger title="Remove vault" text={`Delete the vault and all ${v.cookies} stored cookie(s) from this PC? This cannot be undone. Accounts stay in the list.`} onYes={async () => { await api.vaultRemove(); await refresh(); }} onClose={() => setDlg(null)} />}
    </div>
  );
}
