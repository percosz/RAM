import { useEffect, useState } from "react";
import { Users, Activity, Settings as SettingsIcon, Lock, Unlock, ShieldOff } from "lucide-react";
import { api } from "./api";
import { AppProvider, useApp } from "./store";
import { Toasts } from "./components";
import { AccountsView } from "./views/Accounts";
import { SettingsView } from "./views/Settings";
import { ClientsView } from "./views/Clients";
import { VaultUnlockDialog } from "./dialogs";
import "./styles/ui.css";
import "./styles/app.css";

type View = "accounts" | "clients" | "settings";

function Shell() {
  const { snap, settings, accounts, selected, setSelected, toasts, dismissToast, refresh } = useApp();
  const [view, setView] = useState<View>("accounts");
  const [unlock, setUnlock] = useState(false);
  const vault = snap?.vault;
  // Classic: alyae's single-pane look — a slim top bar instead of the sidebar.
  const classic = settings?.layout === "Classic";
  // Dev hook for screenshots (env vars; no effect in normal use).
  const [hinted, setHinted] = useState(false);
  useEffect(() => {
    if (hinted || !snap) return;
    setHinted(true);
    api.startHint().then(([v, first]) => {
      if (v === "clients" || v === "settings" || v === "accounts") setView(v);
      if (first && snap.accounts.length) setSelected([snap.accounts[0]!.id]);
    }).catch(() => {});
  }, [snap, hinted, setSelected]);
  const valid = accounts.filter((a) => a.sign_in.state === "Verified").length;
  const invalid = accounts.filter((a) => a.sign_in.state === "SignedOut").length;
  const running = accounts.filter((a) => a.client === "Running").length;

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
  const tabs: Array<[View, string, React.ReactNode]> = [["accounts", "Accounts", <Users />], ["clients", "Clients", <Activity />], ["settings", "Settings", <SettingsIcon />]];

  return (
    <div className={`shell ${classic ? "classic" : ""}`}>
      {classic ? (
        <header className="topbar">
          <div className="brand"><span className="logo">R</span><b>RAM</b></div>
          <div className="tabs">
            {tabs.map(([id, name, icon]) => <button key={id} className={`tab ${view === id ? "active" : ""}`} onClick={() => setView(id)}>{icon} {name}</button>)}
          </div>
          <span className="grow" />
          {vaultPill}
        </header>
      ) : (
        <aside className="sidebar">
          <div className="brand"><span className="logo">R</span><b>RAM</b></div>
          {tabs.map(([id, name, icon]) => <button key={id} className={`nav ${view === id ? "active" : ""}`} onClick={() => setView(id)}>{icon} {name}</button>)}
          <div className="spacer" />
          {vaultPill}
        </aside>
      )}
      <main className="content">
        {view === "accounts" && <AccountsView onNeedUnlock={() => setUnlock(true)} />}
        {view === "clients" && <ClientsView />}
        {view === "settings" && <SettingsView onNeedUnlock={() => setUnlock(true)} />}
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
