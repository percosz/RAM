// Typed wrappers over the Tauri commands and events. Shapes mirror
// src-tauri/src/dto.rs and ram-core's serde models.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type SignInState =
  | "Verified" | "SignedOut" | "VerificationRequired" | "CaptchaRequired" | "CheckInBrowser" | "Unknown";

export interface SignInRecord {
  state: SignInState;
  last_check: number | null;
  detail: string;
  last_failed_check: number | null;
}

export interface Account {
  id: number;
  username: string;
  alias: string;
  display_name: string;
  group: string;
  tags: string[];
  notes: string;
  favorite: boolean;
  is_main: boolean;
  roblox_user_id: number | null;
  user_id_confirmed: boolean;
  sign_in: SignInRecord;
  has_cookie: boolean;
  checking: boolean;
  last_used: number | null;
  robux: number | null;
  premium: boolean | null;
  client: string;
  client_pid: number | null;
}

export type ThemeKind =
  | "System" | "Light" | "Dark" | "Mocha" | "Macchiato" | "Frappe" | "Latte"
  | "Nord" | "Dracula" | "Gruvbox" | "RosePine" | "Lineage";

export interface Settings {
  theme: ThemeKind;
  accent: number | null;
  compact: boolean;
  layout: "Modern" | "Classic";
  corner_radius: number;
  direct_launch: boolean;
  multi_instance: boolean;
  launch_concurrency: number;
  launch_spacing_secs: number;
  launch_timeout_secs: number;
  include_main_in_batch: boolean;
  oauth_client_id: string;
  oauth_port: number;
  idle_reminder_minutes: number;
  window: unknown;
  last_import_dir: string;
  default_destination: number | null;
}

export interface Game { place_id: number; universe_id: number | null; name: string; pinned: boolean; builtin: boolean; }

export interface VaultStatus {
  configured: boolean;
  mode: "dpapi" | "password" | null;
  unlocked: boolean;
  cookies: number;
  auto_lock_secs: number;
  require_hello: boolean;
  hello_available: boolean;
}

export interface Snapshot {
  version: string;
  accounts: Account[];
  settings: Settings;
  games: Game[];
  vault: VaultStatus;
  main_account: number | null;
  multi_instance_held: boolean;
  startup_note: string;
  data_dir: string;
}

export interface AccountPatch {
  id: number;
  alias?: string;
  group?: string;
  tags?: string[];
  notes?: string;
  favorite?: boolean;
}

export interface LaunchEvent { account: number; state: string; message: string; }

export interface ClientView { pid: number; account: number | null; account_name: string | null; working_set_mb: number; private_mb: number; }

export interface PlanItem { index: number; kind: string; label: string; detail: string; action: string; selectable: boolean; selected: boolean; }
export interface PlanView { folder: string; items: PlanItem[]; notices: string[]; add: number; update: number; unchanged: number; skip: number; cookies_available: number; }

export const api = {
  startHint: () => invoke<[string, boolean]>("start_hint"),
  snapshot: () => invoke<Snapshot>("snapshot"),
  addAccount: (username: string, alias: string) => invoke<Account>("add_account", { username, alias }),
  updateAccount: (patch: AccountPatch) => invoke<Account>("update_account", { patch }),
  removeAccounts: (ids: number[]) => invoke<void>("remove_accounts", { ids }),
  setMain: (id: number | null) => invoke<void>("set_main", { id }),
  reorderAccounts: (ids: number[]) => invoke<void>("reorder_accounts", { ids }),
  setSettings: (settings: Settings) => invoke<Settings>("set_settings", { settings }),
  openDataFolder: () => invoke<string>("open_data_folder"),

  vaultStatus: () => invoke<VaultStatus>("vault_status"),
  vaultSetupDpapi: () => invoke<VaultStatus>("vault_setup_dpapi"),
  vaultSetupPassword: (password: string) => invoke<string>("vault_setup_password", { password }),
  vaultUnlock: (password: string) => invoke<VaultStatus>("vault_unlock", { password }),
  vaultUnlockRecovery: (code: string) => invoke<VaultStatus>("vault_unlock_recovery", { code }),
  vaultLock: () => invoke<VaultStatus>("vault_lock"),
  vaultChangePassword: (current: string, password: string) => invoke<string>("vault_change_password", { current, password }),
  vaultRemove: () => invoke<VaultStatus>("vault_remove"),
  vaultOptions: (autoLockSecs: number, requireHello: boolean) => invoke<VaultStatus>("vault_options", { autoLockSecs, requireHello }),
  vaultTick: () => invoke<boolean>("vault_tick"),
  cookieSet: (id: number, cookie: string) => invoke<void>("cookie_set", { id, cookie }),
  cookieRemove: (id: number) => invoke<void>("cookie_remove", { id }),
  cookieReveal: (id: number, password?: string) => invoke<string>("cookie_reveal", { id, password: password ?? null }),

  checkAccounts: (ids: number[]) => invoke<void>("check_accounts", { ids }),
  launch: (ids: number[], placeId: number, jobId: string | null) => invoke<void>("launch", { ids, placeId, jobId }),
  follow: (ids: number[], username: string) => invoke<void>("follow", { ids, username }),

  openBrowser: (id: number) => invoke<void>("open_browser", { id }),
  browserCapture: (id: number) => invoke<void>("browser_capture", { id }),
  browserClear: (id: number) => invoke<void>("browser_clear", { id }),

  importScan: (folder: string) => invoke<PlanView>("import_scan", { folder }),
  importApply: (folder: string, selected: number[], withCookies: boolean) => invoke<string>("import_apply", { folder, selected, withCookies }),
  exportAccounts: (folder: string, withCookies: boolean) => invoke<string>("export_accounts", { folder, withCookies }),
  clientsSnapshot: () => invoke<ClientView[]>("clients_snapshot"),
  closeClient: (pid: number) => invoke<void>("close_client", { pid }),

  onAccountUpdated: (cb: (id: number) => void): Promise<UnlistenFn> => listen<number>("account-updated", (e) => cb(e.payload)),
  onLaunchStatus: (cb: (e: LaunchEvent) => void): Promise<UnlistenFn> => listen<LaunchEvent>("launch-status", (e) => cb(e.payload)),
};

/** Human label for a sign-in state. */
export function signInLabel(s: SignInState): string {
  switch (s) {
    case "Verified": return "Valid";
    case "SignedOut": return "Invalid";
    case "VerificationRequired": return "Needs verification";
    case "CaptchaRequired": return "CAPTCHA required";
    case "CheckInBrowser": return "Check in browser";
    default: return "Unknown";
  }
}

/** Relative time such as "3m" / "2h" / "16d", or "" when unknown. */
export function ago(unixSecs: number | null): string {
  if (!unixSecs) return "";
  const d = Math.max(0, Math.floor(Date.now() / 1000) - unixSecs);
  if (d < 60) return "now";
  if (d < 3600) return `${Math.floor(d / 60)}m`;
  if (d < 86400) return `${Math.floor(d / 3600)}h`;
  return `${Math.floor(d / 86400)}d`;
}
