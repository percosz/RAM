// App-wide state: the backend snapshot, selection, toasts, theme application,
// the update offer and the crash counter.

import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { api, type Account, type GraphicsSettings, type LimitSettings, type Settings, type Snapshot } from "./api";
import type { ToastItem } from "./components";

export interface UpdateOffer {
  version: string;
  notes: string;
  date: string;
  state: "available" | "downloading" | "installing" | "dismissed" | "failed";
  progress: number; // 0..1 while downloading
  error: string;
  /** Portable copies cannot self-install; the dialog links to the release instead. */
  portable: boolean;
}

interface Ctx {
  snap: Snapshot | null;
  accounts: Account[];
  settings: Settings | null;
  refresh: () => Promise<void>;
  saveSettings: (patch: Partial<Settings>) => Promise<void>;
  saveGraphics: (patch: Partial<GraphicsSettings>) => Promise<void>;
  saveLimits: (patch: Partial<LimitSettings>) => Promise<void>;
  selected: number[];
  setSelected: (ids: number[]) => void;
  toast: (text: string, tone?: ToastItem["tone"]) => void;
  toasts: ToastItem[];
  dismissToast: (id: number) => void;
  launchStatus: Record<number, { state: string; message: string }>;
  update: UpdateOffer | null;
  checkForUpdate: (manual: boolean) => Promise<string>;
  installUpdate: () => Promise<void>;
  dismissUpdate: () => void;
  crashCount: number;
  clearCrashCount: () => void;
  /** A username as it should be shown: masked when Hide usernames is on. */
  nameOf: (username: string) => string;
}

const AppCtx = createContext<Ctx | null>(null);

export function useApp(): Ctx {
  const c = useContext(AppCtx);
  if (!c) throw new Error("useApp outside provider");
  return c;
}

const THEME_ATTR: Record<string, string> = {
  System: "", Light: "light", Dark: "dark", Mocha: "mocha", Macchiato: "macchiato", Frappe: "frappe", Latte: "latte",
  Nord: "nord", Dracula: "dracula", Gruvbox: "gruvbox", RosePine: "rosepine", Lineage: "lineage",
};

function applyTheme(s: Settings) {
  const root = document.documentElement;
  let name = THEME_ATTR[s.theme] ?? "";
  if (!name) name = window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
  root.setAttribute("data-theme", name);
  root.setAttribute("data-density", (s.density ?? "Normal").toLowerCase());
  if (s.accent != null) {
    const hex = "#" + (s.accent & 0xffffff).toString(16).padStart(6, "0");
    root.style.setProperty("--accent", hex);
    // Readable text on the accent: pick by luminance.
    const r = (s.accent >> 16) & 255, g = (s.accent >> 8) & 255, b = s.accent & 255;
    root.style.setProperty("--accent-text", (r * 299 + g * 587 + b * 114) / 1000 > 140 ? "#101214" : "#ffffff");
  } else {
    root.style.removeProperty("--accent");
    root.style.removeProperty("--accent-text");
  }
  root.style.setProperty("--radius", `${Math.max(0, Math.min(16, s.corner_radius)) + 4}px`);
  root.style.setProperty("--radius-sm", `${Math.max(0, Math.min(16, s.corner_radius))}px`);
}

/** "Thornwick_Eli" → "Tho********" when hiding, like niccs' manager. */
export function maskName(username: string, hide: boolean): string {
  if (!hide || !username) return username;
  return username.slice(0, 3) + "********";
}

export function AppProvider({ children }: { children: ReactNode }) {
  const [snap, setSnap] = useState<Snapshot | null>(null);
  const [selected, setSelected] = useState<number[]>([]);
  const [toasts, setToasts] = useState<ToastItem[]>([]);
  const [launchStatus, setLaunchStatus] = useState<Record<number, { state: string; message: string }>>({});
  const [update, setUpdate] = useState<UpdateOffer | null>(null);
  const [crashCount, setCrashCount] = useState(0);
  const toastId = useRef(1);

  const refresh = useCallback(async () => {
    try {
      const s = await api.snapshot();
      setSnap(s);
      applyTheme(s.settings);
      setSelected((sel) => sel.filter((id) => s.accounts.some((a) => a.id === id)));
    } catch (e) {
      console.error(e);
    }
  }, []);

  const toast = useCallback((text: string, tone?: ToastItem["tone"]) => {
    const id = toastId.current++;
    setToasts((t) => [...t, { id, text, tone }]);
    window.setTimeout(() => setToasts((t) => t.filter((x) => x.id !== id)), 5000);
  }, []);
  const dismissToast = useCallback((id: number) => setToasts((t) => t.filter((x) => x.id !== id)), []);

  const saveSettings = useCallback(async (patch: Partial<Settings>) => {
    if (!snap) return;
    const next = { ...snap.settings, ...patch };
    applyTheme(next);
    const saved = await api.setSettings(next);
    setSnap((s) => (s ? { ...s, settings: saved } : s));
  }, [snap]);

  const saveGraphics = useCallback(async (patch: Partial<GraphicsSettings>) => {
    if (!snap) return;
    const saved = await api.setGraphics({ ...snap.graphics, ...patch });
    setSnap((s) => (s ? { ...s, graphics: saved } : s));
  }, [snap]);

  const saveLimits = useCallback(async (patch: Partial<LimitSettings>) => {
    if (!snap) return;
    const saved = await api.setLimits({ ...snap.limits, ...patch });
    setSnap((s) => (s ? { ...s, limits: saved } : s));
  }, [snap]);

  /** Asks GitHub for a newer release. Returns a one-line result for Settings. */
  const checkForUpdate = useCallback(async (manual: boolean): Promise<string> => {
    try {
      const u = await api.updateCheck();
      if (!u) { if (manual) setUpdate(null); return "RAM is up to date."; }
      const portable = (await api.installKind().catch(() => "installed")) === "portable";
      setUpdate((cur) => cur && cur.version === u.version && cur.state === "dismissed" && !manual
        ? cur
        : { version: u.version, notes: u.notes, date: u.date, state: "available", progress: 0, error: "", portable });
      return `RAM ${u.version} is available.`;
    } catch (e) {
      return `Could not check for updates: ${String(e)}`;
    }
  }, []);

  const installUpdate = useCallback(async () => {
    if (!update || update.portable) return;
    setUpdate((u) => (u ? { ...u, state: "downloading", progress: 0, error: "" } : u));
    try {
      await api.updateInstall();
      // On Windows the installer takes over and RAM exits here.
    } catch (e) {
      setUpdate((u) => (u ? { ...u, state: "failed", error: String(e) } : u));
    }
  }, [update]);

  const dismissUpdate = useCallback(() => setUpdate((u) => (u ? { ...u, state: "dismissed" } : u)), []);
  const clearCrashCount = useCallback(() => setCrashCount(0), []);

  useEffect(() => {
    void refresh();
    // Headshots: public API, refreshed when missing or a day old.
    api.refreshAvatars().catch(() => {});
    const unsubs: Array<() => void> = [];
    // A batch check fires one event per account; take one snapshot for the burst.
    let pending: number | null = null;
    api.onAccountUpdated(() => {
      if (pending != null) return;
      pending = window.setTimeout(() => { pending = null; void refresh(); }, 120);
    }).then((u) => unsubs.push(u));
    // A launch line is cleared when the client is seen (Running) or gone
    // (Exited), and "Client starting…" never outlives two minutes.
    const expiry = new Map<number, number>();
    const clear = (account: number) => setLaunchStatus((m) => { const n = { ...m }; delete n[account]; return n; });
    api.onLaunchStatus((e) => {
      const old = expiry.get(e.account); if (old) window.clearTimeout(old);
      if (e.state === "Running" || e.state === "Exited") { clear(e.account); return; }
      setLaunchStatus((m) => ({ ...m, [e.account]: { state: e.state, message: e.message } }));
      if (e.state === "Failed") toast(e.message, "bad");
      expiry.set(e.account, window.setTimeout(() => clear(e.account), e.state === "Failed" ? 30_000 : 120_000));
    }).then((u) => unsubs.push(u));
    api.onCrashesChanged((n) => setCrashCount((c) => c + n)).then((u) => unsubs.push(u));
    api.onUpdateProgress((p) => setUpdate((u) => (u ? { ...u, state: p.done ? "installing" : "downloading", progress: p.done ? 1 : p.total ? Math.min(1, p.got / p.total) : 0 } : u))).then((u) => unsubs.push(u));
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const onScheme = () => { void refresh(); };
    mq.addEventListener("change", onScheme);
    // Auto-lock tick for master-password vaults.
    const tick = window.setInterval(async () => {
      try { if (await api.vaultTick()) { toast("Vault locked after inactivity.", "warn"); void refresh(); } } catch { /* ignore */ }
    }, 20_000);
    return () => { unsubs.forEach((u) => u()); mq.removeEventListener("change", onScheme); window.clearInterval(tick); };
  }, [refresh, toast]);

  // An update check right after start, then every 6 hours, when enabled.
  // Keyed on the setting alone: keyed on the whole snapshot, the first
  // refresh after start (the account checks) cancelled the timer and the
  // check never ran.
  const autoCheck = !!snap?.settings.check_updates;
  useEffect(() => {
    if (!autoCheck) return;
    const t = window.setTimeout(() => { void checkForUpdate(false); }, 1500);
    const every = window.setInterval(() => { void checkForUpdate(false); }, 6 * 3600 * 1000);
    return () => { window.clearTimeout(t); window.clearInterval(every); };
  }, [autoCheck, checkForUpdate]);

  const hideNames = !!snap?.settings.hide_usernames;
  const nameOf = useCallback((username: string) => maskName(username, hideNames), [hideNames]);

  const value = useMemo<Ctx>(() => ({
    snap, accounts: snap?.accounts ?? [], settings: snap?.settings ?? null, refresh, saveSettings, saveGraphics, saveLimits,
    selected, setSelected, toast, toasts, dismissToast, launchStatus,
    update, checkForUpdate, installUpdate, dismissUpdate, crashCount, clearCrashCount, nameOf,
  }), [nameOf, saveLimits, snap, refresh, saveSettings, saveGraphics, selected, toast, toasts, dismissToast, launchStatus, update, checkForUpdate, installUpdate, dismissUpdate, crashCount, clearCrashCount]);

  return <AppCtx.Provider value={value}>{children}</AppCtx.Provider>;
}
