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
  avatar_url: string | null;
  fps_cap: number | null;
  full_quality: boolean;
  client: string;
  client_pid: number | null;
}

export type ThemeKind =
  | "System" | "Light" | "Dark" | "Mocha" | "Macchiato" | "Frappe" | "Latte"
  | "Nord" | "Dracula" | "Gruvbox" | "RosePine" | "Lineage";

export type Density = "Compact" | "Normal" | "Comfortable" | "Large";

export interface Settings {
  theme: ThemeKind;
  accent: number | null;
  density: Density;
  corner_radius: number;
  multi_instance: boolean;
  launch_spacing_secs: number;
  launch_timeout_secs: number;
  include_main_in_batch: boolean;
  check_updates: boolean;
  seen_version: string;
  window: unknown;
  last_import_dir: string;
}

export interface Game { place_id: number; universe_id: number | null; name: string; pinned: boolean; builtin: boolean; creator: string; fetched_at: number | null; }

export interface VaultStatus {
  configured: boolean;
  mode: "dpapi" | "password" | null;
  unlocked: boolean;
  cookies: number;
  auto_lock_secs: number;
  require_hello: boolean;
  hello_available: boolean;
}

export type Msaa = "Default" | "X1" | "X2" | "X4";
export type Texture = "Default" | "Lowest" | "Low" | "Medium" | "High";
export type RenderMode = "Default" | "D3D11" | "Vulkan" | "OpenGl";
export interface RenderFlags { msaa: Msaa; texture: Texture; mode: RenderMode; gray_sky: boolean; no_grass: boolean; lowest_frm: boolean; pause_voxelizer: boolean; }
export interface GraphicsSettings {
  enabled: boolean;
  fps_cap: number;
  low_graphics: boolean;
  lighter_rendering: boolean;
  render: RenderFlags;
  custom_flags: Record<string, string>;
}

export interface Snapshot {
  version: string;
  accounts: Account[];
  settings: Settings;
  graphics: GraphicsSettings;
  games: Game[];
  vault: VaultStatus;
  main_account: number | null;
  multi_instance_held: boolean;
  startup_note: string;
  data_dir: string;
  /** Name of the browser Open Browser uses ("Brave", "Chrome"…), or "" when none is usable. */
  browser_name: string;
  /** How launches reach Roblox: "Froststrap", "Bloxstrap", … or "Roblox". */
  launcher_name: string;
  /** "owner/RAM" on GitHub, where releases come from. */
  github_repo: string;
}

export interface Adopted { account: Account; added: boolean; }
export interface ManyResult { added: number; refreshed: number; repeated: number; failed: number; problems: string[]; }
export type SignInPoll = { state: "waiting" } | { state: "closed" } | ({ state: "done" } & Adopted);

export interface UpdateInfo { version: string; notes: string; date: string; }
export interface UpdateProgress { got: number; total: number; done: boolean; }

export interface AccountPatch {
  id: number;
  alias?: string;
  group?: string;
  tags?: string[];
  notes?: string;
  favorite?: boolean;
  full_quality?: boolean;
  fps_cap?: number | null;
  clear_fps_cap?: boolean;
}

export interface LaunchEvent { account: number; state: string; message: string; }

export interface ClientView { pid: number; account: number | null; account_name: string | null; working_set_mb: number; private_mb: number; }

export type EndKind = "Closed" | "Disconnected" | "Crashed" | "Teleported" | "Replaced" | "SignedOut";
export interface CrashRecord {
  id: number;
  account: number | null;
  username: string;
  kind: EndKind;
  at: number;
  uptime_secs: number;
  reason: string;
  place_id: number | null;
  place_name: string;
  has_log: boolean;
  pid: number;
}
export interface LogLine { level: "debug" | "info" | "warn" | "error"; text: string; }

export interface PlanItem { index: number; kind: string; label: string; detail: string; action: string; selectable: boolean; selected: boolean; }
export interface PlanView { folder: string; items: PlanItem[]; notices: string[]; add: number; update: number; unchanged: number; skip: number; cookies_available: number; }

export const api = {
  startHint: () => invoke<[string, boolean, string]>("start_hint"),
  snapshot: () => invoke<Snapshot>("snapshot"),
  addAccount: (username: string, alias: string) => invoke<Account>("add_account", { username, alias }),
  addByCookie: (cookie: string, alias: string) => invoke<Adopted>("add_by_cookie", { cookie, alias }),
  addMany: (text: string) => invoke<ManyResult>("add_many", { text }),
  signinStart: () => invoke<string>("signin_start"),
  signinPoll: () => invoke<SignInPoll>("signin_poll"),
  signinCancel: () => invoke<void>("signin_cancel"),
  onAddManyProgress: (cb: (p: { done: number; total: number }) => void): Promise<UnlistenFn> => listen<{ done: number; total: number }>("add-many-progress", (e) => cb(e.payload)),
  updateAccount: (patch: AccountPatch) => invoke<Account>("update_account", { patch }),
  removeAccounts: (ids: number[]) => invoke<void>("remove_accounts", { ids }),
  setMain: (id: number | null) => invoke<void>("set_main", { id }),
  reorderAccounts: (ids: number[]) => invoke<void>("reorder_accounts", { ids }),
  setSettings: (settings: Settings) => invoke<Settings>("set_settings", { settings }),
  setGraphics: (graphics: GraphicsSettings) => invoke<GraphicsSettings>("set_graphics", { graphics }),
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
  refreshAvatars: (force = false) => invoke<void>("refresh_avatars", { force }),
  placeInfo: (placeId: number) => invoke<Game | null>("place_info", { placeId }),
  launch: (ids: number[], placeId: number, jobId: string | null) => invoke<void>("launch", { ids, placeId, jobId }),
  follow: (ids: number[], username: string) => invoke<void>("follow", { ids, username }),

  openBrowser: (id: number) => invoke<string>("open_browser", { id }),
  browserCapture: (id: number) => invoke<void>("browser_capture", { id }),
  browserClear: (id: number) => invoke<void>("browser_clear", { id }),

  importScan: (folder: string) => invoke<PlanView>("import_scan", { folder }),
  importApply: (folder: string, selected: number[], withCookies: boolean) => invoke<string>("import_apply", { folder, selected, withCookies }),
  exportAccounts: (folder: string, withCookies: boolean) => invoke<string>("export_accounts", { folder, withCookies }),
  clientsSnapshot: () => invoke<ClientView[]>("clients_snapshot"),
  closeClient: (pid: number) => invoke<void>("close_client", { pid }),

  crashes: () => invoke<CrashRecord[]>("crashes_list"),
  crashesClear: () => invoke<void>("crashes_clear"),
  crashOpenLog: (id: number) => invoke<void>("crash_open_log", { id }),
  crashesExport: (path: string) => invoke<void>("crashes_export", { path }),
  logTail: (lines: number) => invoke<LogLine[]>("log_tail", { lines }),
  openLogFolder: () => invoke<void>("open_log_folder"),
  diagnosticsText: () => invoke<string>("diagnostics_text"),

  updateCheck: () => invoke<UpdateInfo | null>("update_check"),
  updateInstall: () => invoke<void>("update_install"),
  installKind: () => invoke<"installed" | "portable">("install_kind"),
  markSeenVersion: () => invoke<void>("mark_seen_version"),
  releaseNotes: () => invoke<string>("release_notes"),
  openUrl: (url: string) => invoke<void>("open_url", { url }),

  onAccountUpdated: (cb: (id: number) => void): Promise<UnlistenFn> => listen<number>("account-updated", (e) => cb(e.payload)),
  onLaunchStatus: (cb: (e: LaunchEvent) => void): Promise<UnlistenFn> => listen<LaunchEvent>("launch-status", (e) => cb(e.payload)),
  onCrashesChanged: (cb: (n: number) => void): Promise<UnlistenFn> => listen<number>("crashes-changed", (e) => cb(e.payload)),
  onUpdateProgress: (cb: (p: UpdateProgress) => void): Promise<UnlistenFn> => listen<UpdateProgress>("update-progress", (e) => cb(e.payload)),
};

/** Human label for how a client ended. */
export function endKindLabel(k: EndKind): string {
  switch (k) {
    case "Closed": return "Closed";
    case "Disconnected": return "Disconnected by Roblox";
    case "Crashed": return "Crashed";
    case "Teleported": return "Teleported";
    case "Replaced": return "Replaced by another client";
    case "SignedOut": return "Sign-in refused";
  }
}

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
