import { useEffect, useState } from "react";
import { Users, Activity, Settings as SettingsIcon, Lock, Unlock, ShieldOff, Stethoscope, ArrowDownToLine } from "lucide-react";
import { api } from "./api";
import { AppProvider, useApp } from "./store";
import { Toasts } from "./components";
import { AccountsView } from "./views/Accounts";
import { SettingsView } from "./views/Settings";
import { ClientsView } from "./views/Clients";
import { DiagnosticsView } from "./views/Diagnostics";
import { UpdateDialog, WhatsNewDialog } from "./views/Updates";
import { VaultUnlockDialog } from "./dialogs";
import "./styles/ui.css";
import "./styles/app.css";

type View = "accounts" | "clients" | "diagnostics" | "settings";

function Shell() {
  const { snap, accounts, selected, setSelected, toasts, dismissToast, refresh, update, crashCount } = useApp();
  const [view, setView] = useState<View>("accounts");
  const [unlock, setUnlock] = useState(false);
  const [showUpdate, setShowUpdate] = useState(false);
  const vault = snap?.vault;
  // Dev hook for screenshots (env vars; no effect in normal use).
  const [hinted, setHinted] = useState(false);
  useEffect(() => {
    if (hinted || !snap) return;
    setHinted(true);
    api.startHint().then(([v, first, dialog]) => {
      if (v === "clients" || v === "settings" || v === "accounts" || v === "diagnostics") setView(v);
      if (first && snap.accounts.length) setSelected([snap.accounts[0]!.id]);
      if (dialog) window.dispatchEvent(new CustomEvent("ram-start-dialog", { detail: dialog }));
    }).catch(() => {});
  }, [snap, hinted, setSelected]);
  const valid = accounts.filter((a) => a.sign_in.state === "Verified").length;
  const invalid = accounts.filter((a) => a.sign_in.state === "SignedOut").length;
  const running = accounts.filter((a) => a.client === "Running").length;
  // "What's new" once after an update: the stored seen_version lags the running one.
  const whatsNew = !!snap && snap.settings.seen_version !== "" && snap.settings.seen_version !== snap.version;

  const vaultPill = vault && (
    <div
      className={`vault-pill ${!vault.configured ? "" : vault.unlocked ? "unlocked" : "locked"}`}
      onClick={() => (vault.configured && !vault.unlocked ? setUnlock(true) : setView("settings"))}
      title={vault.configured ? (vault.unlocked ? "Vault unlocked" : "Vault locked — click to unlock") : "No cookie vault yet"}
    >
      {!vault.configured ? <ShieldOff /> : vault.unlocked ? <Unlock /> : <Lock />}
      <span className="grow ellipsis">
        {!vault.configured ? "No vault" : vault.unlocked ? `Vault · ${vault.cookies} cookie${vault.cookies === 1 ? "" : "s"}` : "Vault locked"}
      </span>
    </div>
  );
  const tabs: Array<[View, string, React.ReactNode]> = [["accounts", "Accounts", <Users />], ["clients", "Clients", <Activity />], ["diagnostics", "Diagnostics", <Stethoscope />], ["settings", "Settings", <SettingsIcon />]];

  return (
    <div className="shell">
      <aside className="sidebar">
        <div className="brand"><span className="logo">R</span><b>RAM</b></div>
        {tabs.map(([id, name, icon]) => (
          <button key={id} className={`nav ${view === id ? "active" : ""}`} onClick={() => setView(id)}>
            {icon} {name}{id === "diagnostics" && crashCount > 0 && <span className="pip" title={`${crashCount} new crash record${crashCount === 1 ? "" : "s"}`} />}
          </button>
        ))}
        <div className="spacer" />
        {update && update.state !== "dismissed" && (
          <div className="update-pill" onClick={() => setShowUpdate(true)} title="A new RAM is available">
            <ArrowDownToLine />
            <span className="grow ellipsis">{update.state === "installing" ? `Installing ${update.version}…` : update.state === "downloading" ? `Downloading ${update.version}…` : `RAM ${update.version} available`}</span>
          </div>
        )}
        {vaultPill}
      </aside>
      <main className="content">
        {view === "accounts" && <AccountsView onNeedUnlock={() => setUnlock(true)} />}
        {view === "clients" && <ClientsView />}
        {view === "diagnostics" && <DiagnosticsView />}
        {view === "settings" && <SettingsView onNeedUnlock={() => setUnlock(true)} onShowUpdate={() => setShowUpdate(true)} />}
      </main>
      <footer className="statusbar">
        <span><b className="accent">{selected.length}</b> selected</span>
        <span><b>{accounts.length}</b> accounts</span>
        <span><b className="good">{valid}</b> valid</span>
        <span><b className="good">{running}</b> in game</span>
        {snap?.startup_note && <span className="warn ellipsis" title={snap.startup_note}>{snap.startup_note}</span>}
        <span className="legend">
          <span className="good"><i className="dot" /> valid</span>
          <span className="bad"><i className="dot" /> invalid{invalid ? ` (${invalid})` : ""}</span>
          <span className="warn"><i className="dot" /> needs check</span>
          <span className="accent"><i className="dot" /> running</span>
        </span>
      </footer>
      <Toasts items={toasts} dismiss={dismissToast} />
      {unlock && <VaultUnlockDialog onClose={() => { setUnlock(false); void refresh(); }} />}
      {showUpdate && update && <UpdateDialog onClose={() => setShowUpdate(false)} />}
      {whatsNew && !showUpdate && <WhatsNewDialog />}
    </div>
  );
}

export default function App() {
  return (
    <AppProvider>
      <Shell />
    </AppProvider>
  );
}
