// App-wide state: the backend snapshot, selection, toasts, and theme application.

import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { api, type Account, type Settings, type Snapshot } from "./api";
import type { ToastItem } from "./components";

interface Ctx {
  snap: Snapshot | null;
  accounts: Account[];
  settings: Settings | null;
  refresh: () => Promise<void>;
  saveSettings: (patch: Partial<Settings>) => Promise<void>;
  selected: number[];
  setSelected: (ids: number[]) => void;
  toast: (text: string, tone?: ToastItem["tone"]) => void;
  toasts: ToastItem[];
  dismissToast: (id: number) => void;
  launchStatus: Record<number, { state: string; message: string }>;
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
  root.setAttribute("data-density", s.compact ? "compact" : "normal");
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

export function AppProvider({ children }: { children: ReactNode }) {
  const [snap, setSnap] = useState<Snapshot | null>(null);
  const [selected, setSelected] = useState<number[]>([]);
  const [toasts, setToasts] = useState<ToastItem[]>([]);
  const [launchStatus, setLaunchStatus] = useState<Record<number, { state: string; message: string }>>({});
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

  useEffect(() => {
    void refresh();
    const unsubs: Array<() => void> = [];
    api.onAccountUpdated(() => { void refresh(); }).then((u) => unsubs.push(u));
    api.onLaunchStatus((e) => {
      setLaunchStatus((m) => ({ ...m, [e.account]: { state: e.state, message: e.message } }));
      if (e.state === "Failed") toast(e.message, "bad");
    }).then((u) => unsubs.push(u));
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const onScheme = () => { void refresh(); };
    mq.addEventListener("change", onScheme);
    // Auto-lock tick for master-password vaults.
    const tick = window.setInterval(async () => {
      try { if (await api.vaultTick()) { toast("Vault locked after inactivity.", "warn"); void refresh(); } } catch { /* ignore */ }
    }, 20_000);
    return () => { unsubs.forEach((u) => u()); mq.removeEventListener("change", onScheme); window.clearInterval(tick); };
  }, [refresh, toast]);

  const value = useMemo<Ctx>(() => ({
    snap, accounts: snap?.accounts ?? [], settings: snap?.settings ?? null, refresh, saveSettings,
    selected, setSelected, toast, toasts, dismissToast, launchStatus,
  }), [snap, refresh, saveSettings, selected, toast, toasts, dismissToast, launchStatus]);

  return <AppCtx.Provider value={value}>{children}</AppCtx.Provider>;
}
