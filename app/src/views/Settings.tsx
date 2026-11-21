import { useState } from "react";
import { open as pickDialog } from "@tauri-apps/plugin-dialog";
import { api, type ThemeKind } from "../api";
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

export function SettingsView({ onNeedUnlock }: { onNeedUnlock: () => void }) {
  const { snap, settings, saveSettings, refresh, toast } = useApp();
  const [dlg, setDlg] = useState<null | "setup" | "change" | "removeVault" | "import">(null);
  const [exportCookies, setExportCookies] = useState(false);
  if (!snap || !settings) return null;
  const v = snap.vault;
  const hex = settings.accent != null ? "#" + (settings.accent & 0xffffff).toString(16).padStart(6, "0") : "";

  const exportNow = async () => {
    if (exportCookies && !v.unlocked) { onNeedUnlock(); return; }
    const dir = await pickDialog({ directory: true, title: "Choose where to write AccountData.json" });
    if (!dir || Array.isArray(dir)) return;
    try { toast(await api.exportAccounts(dir, exportCookies), "good"); } catch (e) { toast(String(e), "bad"); }
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
            <span>Layout</span>
            <div className="seg">
              <button type="button" className={settings.layout === "Modern" ? "active" : ""} onClick={() => saveSettings({ layout: "Modern" })}>Modern</button>
              <button type="button" className={settings.layout === "Classic" ? "active" : ""} onClick={() => saveSettings({ layout: "Classic" })}>Classic</button>
            </div>
            <span />
            <div className="hint">Modern: sidebar and a details pane. Classic: one list with a top bar, like alyae's manager.</div>
          </div>
          <div className="kv">
            <span>Corners</span>
            <input type="range" min={0} max={16} value={settings.corner_radius} onChange={(e) => saveSettings({ corner_radius: Number(e.target.value) })} />
          </div>
          <Toggle checked={settings.compact} onChange={(c) => saveSettings({ compact: c })} label="Compact rows" />
        </div>
      </section>

      <section className="card">
        <div className="card-h">Launching</div>
        <div className="card-b">
          <Toggle checked={settings.multi_instance} onChange={(c) => saveSettings({ multi_instance: c })} label="Run several Roblox clients at once"
            hint={`Holds Roblox's single-instance lock so more than one client can run; never modifies the client. ${snap.multi_instance_held ? "Active." : settings.multi_instance ? "Another program already holds the lock." : ""}`} />
          <div className="kv">
            <span>Seconds between launches</span>
            <input className="input" type="number" min={0} max={120} value={settings.launch_spacing_secs} onChange={(e) => saveSettings({ launch_spacing_secs: Math.max(0, Math.min(120, Number(e.target.value) || 0)) })} />
          </div>
          <Toggle checked={settings.include_main_in_batch} onChange={(c) => saveSettings({ include_main_in_batch: c })} label="Include Main in Select All" />
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
        <div className="card-h">About</div>
        <div className="card-b">
          <div>RAM {snap.version} — a rewrite of <b>Roblox Account Manager</b> by ic3w0lf22, GPL‑3.0.</div>
          <div className="hint">Cookie check modelled on RoALT (MIT); embedded browser model from alyae (GPL‑3.0); layout cues from niccs' RAM v4 (GPL‑3.0). Not affiliated with Roblox Corporation. See NOTICE.md.</div>
        </div>
      </section>

      {dlg === "setup" && <VaultSetupDialog onClose={() => { setDlg(null); void refresh(); }} />}
      {dlg === "change" && <ChangePasswordDialog onClose={() => setDlg(null)} />}
      {dlg === "import" && <ImportDialog onClose={() => setDlg(null)} />}
      {dlg === "removeVault" && <ConfirmDialog danger title="Remove vault" text={`Delete the vault and all ${v.cookies} stored cookie(s) from this PC? This cannot be undone. Accounts stay in the list.`} onYes={async () => { await api.vaultRemove(); await refresh(); }} onClose={() => setDlg(null)} />}
    </div>
  );
}
