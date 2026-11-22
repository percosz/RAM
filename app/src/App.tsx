import { useEffect, useRef, useState } from "react";
import { Users, Activity, Settings as SettingsIcon, Lock, Unlock, ShieldOff, Stethoscope, ArrowDownToLine } from "lucide-react";
import { api } from "./api";
import { AppProvider, useApp } from "./store";
import { Toasts } from "./components";
import { AccountsView } from "./views/Accounts";
import { SettingsView } from "./views/Settings";
import { ClientsView } from "./views/Clients";
import { DiagnosticsView } from "./views/Diagnostics";
import { UpdateDialog, WhatsNewDialog } from "./views/Updates";
import { Tour } from "./views/Tour";
import { VaultUnlockDialog } from "./dialogs";
import "./styles/ui.css";
import "./styles/app.css";
import "./styles/themes.css";
import "./styles/themes2.css";

type View = "accounts" | "clients" | "diagnostics" | "settings";

function Shell() {
  const { snap, accounts, selected, setSelected, toasts, dismissToast, refresh, update, crashCount, saveSettings } = useApp();
  const [view, setView] = useState<View>("accounts");
  const [unlock, setUnlock] = useState(false);
  const [showUpdate, setShowUpdate] = useState(false);
  // The tutorial: the step it opens at, or null while closed. It opens by
  // itself the first time RAM runs and from the bottom of Settings.
  const [tour, setTour] = useState<number | null>(null);
  const tourOffered = useRef(false);
  useEffect(() => {
    if (!snap || tourOffered.current) return;
    tourOffered.current = true;
    if (!snap.settings.tour_seen) setTour((t) => t ?? 0);
  }, [snap]);
  const endTour = () => {
    setTour(null);
    setView("accounts");
    if (snap && !snap.settings.tour_seen) void saveSettings({ tour_seen: true });
  };
  const vault = snap?.vault;
  // A new version found at launch is announced once: Update now or Later.
  // One found hours into a session only lights the sidebar pill, so a
  // dialog never lands on top of whatever the user is doing.
  const born = useRef(Date.now());
  const announced = useRef(false);
  useEffect(() => {
    if (announced.current || !update || update.state !== "available") return;
    // The tutorial goes first; the offer follows when it closes.
    if (tour !== null) return;
    if (Date.now() - born.current > 300_000) return;
    announced.current = true;
    setShowUpdate(true);
  }, [update, tour]);
  // The webview's own right-click menu (Back, Refresh, Print) has no place
  // in an app; text fields keep theirs for copy and paste.
  useEffect(() => {
    const block = (e: MouseEvent) => { const el = e.target as HTMLElement; if (!el.closest("input, textarea")) e.preventDefault(); };
    window.addEventListener("contextmenu", block);
    return () => window.removeEventListener("contextmenu", block);
  }, []);
  // Dev hook for screenshots (env vars; no effect in normal use).
  const [hinted, setHinted] = useState(false);
  useEffect(() => {
    if (hinted || !snap) return;
    setHinted(true);
    api.startHint().then(([v, first, dialog]) => {
      if (v === "clients" || v === "settings" || v === "accounts" || v === "diagnostics") setView(v);
      if (first && snap.accounts.length) setSelected([snap.accounts[0]!.id]);
      if (dialog.startsWith("tour")) setTour(Number(dialog.split(":")[1] ?? 0) || 0);
      else if (dialog === "bottom") window.setTimeout(() => document.querySelector(".settings")?.scrollTo(0, 1e6), 400);
      else if (dialog) window.dispatchEvent(new CustomEvent("ram-start-dialog", { detail: dialog }));
    }).catch(() => {});
  }, [snap, hinted, setSelected]);
  const valid = accounts.filter((a) => a.sign_in.state === "Verified").length;
  const invalid = accounts.filter((a) => a.sign_in.state === "SignedOut").length;
  const running = accounts.filter((a) => a.client === "Running").length;
  // "What's new" once after an update: the stored seen_version lags the running one.
  // (A fresh install starts with seen_version set to its own version; an
  // older data file with none recorded gets the notes once.)
  const whatsNew = !!snap && snap.settings.seen_version !== snap.version;

  const vaultPill = vault && (
    <div
      data-tour="vault"
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
        <div className="brand">
          <svg className="mark" viewBox="0 0 512 512" aria-hidden>
            <rect x="16" y="16" width="480" height="480" rx="112" className="mark-bg" />
            <circle cx="148" cy="156" r="44" className="mark-on" /><rect x="222" y="124" width="198" height="64" rx="32" className="mark-on" />
            <circle cx="148" cy="256" r="44" className="mark-off" /><rect x="222" y="224" width="198" height="64" rx="32" className="mark-off" />
            <circle cx="148" cy="356" r="44" className="mark-off" /><rect x="222" y="324" width="138" height="64" rx="32" className="mark-off" />
          </svg>
          <b>RAM</b>
        </div>
        {tabs.map(([id, name, icon]) => (
          <button key={id} data-tour={`nav-${id}`} className={`nav ${view === id ? "active" : ""}`} onClick={() => setView(id)}>
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
        {view === "settings" && <SettingsView onNeedUnlock={() => setUnlock(true)} onShowUpdate={() => setShowUpdate(true)} onShowTour={() => setTour(0)} />}
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
      {showUpdate && update && tour === null && <UpdateDialog onClose={() => setShowUpdate(false)} />}
      {whatsNew && !showUpdate && tour === null && <WhatsNewDialog />}
      {tour !== null && <Tour key={tour} start={tour} setView={setView} onDone={endTour} />}
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
