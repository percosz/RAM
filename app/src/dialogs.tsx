// Modal flows: add account, set/reveal cookie, vault setup/unlock/change, import, confirm.

import { useEffect, useState, type ReactNode } from "react";
import { open as pickDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { Download, ExternalLink } from "lucide-react";
import { api, type BackupInfo, type FoundFile, type PlanView, type QueueEvent } from "./api";
import { Btn, Field, Modal, Toggle } from "./components";
import { useApp } from "./store";

function errText(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
}

export function ConfirmDialog({ title, text, danger, onYes, onClose }: { title: string; text: string; danger?: boolean; onYes: () => void; onClose: () => void }) {
  return (
    <Modal title={title} onClose={onClose} footer={<><Btn onClick={onClose}>Cancel</Btn><Btn kind={danger ? "danger" : "primary"} onClick={() => { onYes(); onClose(); }}>{danger ? "Remove" : "OK"}</Btn></>}>
      <p style={{ margin: 0 }}>{text}</p>
    </Modal>
  );
}

/** Writes everything RAM holds to one password-protected file. */
export function BackupDialog({ onClose }: { onClose: () => void }) {
  const { toast, snap } = useApp();
  const [pw, setPw] = useState(""); const [pw2, setPw2] = useState("");
  const [master, setMaster] = useState(""); const [askMaster, setAskMaster] = useState(false);
  const [busy, setBusy] = useState(false); const [err, setErr] = useState("");
  const ok = pw.length >= 8 && pw === pw2 && (!askMaster || master.length > 0);
  const run = async () => {
    setErr("");
    const path = await saveDialog({ title: "Save the RAM backup", defaultPath: `RAM-backup-${new Date().toISOString().slice(0, 10)}.rambackup`, filters: [{ name: "RAM backup", extensions: ["rambackup"] }] });
    if (!path) return;
    setBusy(true);
    try { toast(await api.backupCreate(path, pw, askMaster ? master : null), "good"); onClose(); }
    catch (e) { const t = errText(e); if (t === "PASSWORD_REQUIRED") setAskMaster(true); else setErr(t); }
    finally { setBusy(false); }
  };
  return (
    <Modal title="Back up everything" onClose={busy ? () => {} : onClose} footer={<><Btn onClick={onClose} disabled={busy}>Cancel</Btn><Btn kind="primary" disabled={!ok || busy} onClick={run}>{busy ? "Encrypting…" : "Choose where to save…"}</Btn></>}>
      <p className="muted" style={{ margin: 0 }}>
        One file with your {snap?.accounts.length ?? 0} account{snap?.accounts.length === 1 ? "" : "s"}, settings, and every stored cookie and saved password, encrypted with a password you choose here. It can be restored on any PC.
      </p>
      <Field label="Backup password (8 characters or more)"><input className="input" type="password" autoFocus value={pw} onChange={(e) => setPw(e.target.value)} /></Field>
      <Field label="Again"><input className="input" type="password" value={pw2} onChange={(e) => setPw2(e.target.value)} onKeyDown={(e) => e.key === "Enter" && ok && run()} /></Field>
      {pw2.length > 0 && pw !== pw2 && <div className="bad">The two passwords are different.</div>}
      {askMaster && <Field label="Your vault's master password"><input className="input" type="password" autoFocus value={master} onChange={(e) => setMaster(e.target.value)} onKeyDown={(e) => e.key === "Enter" && ok && run()} /></Field>}
      <div className="hint">Anyone with this file and its password can sign in as your accounts, so keep both safe. A forgotten backup password cannot be recovered.</div>
      {err && <div className="bad">{err}</div>}
    </Modal>
  );
}

/** Replaces what RAM holds with a backup file, after showing what is in it. */
export function RestoreDialog({ onClose }: { onClose: () => void }) {
  const { toast, snap } = useApp();
  const [path, setPath] = useState(""); const [pw, setPw] = useState("");
  const [info, setInfo] = useState<BackupInfo | null>(null);
  const [busy, setBusy] = useState(false); const [err, setErr] = useState("");
  const pick = async () => {
    const p = await pickDialog({ title: "Choose a RAM backup", filters: [{ name: "RAM backup", extensions: ["rambackup"] }, { name: "All files", extensions: ["*"] }] });
    if (p && !Array.isArray(p)) { setPath(p); setInfo(null); setErr(""); }
  };
  const open = async () => {
    setErr(""); setBusy(true);
    try { setInfo(await api.backupInspect(path, pw)); } catch (e) { setErr(errText(e)); } finally { setBusy(false); }
  };
  const restore = async () => {
    setErr(""); setBusy(true);
    try { toast(await api.backupRestore(path, pw), "good"); onClose(); } catch (e) { setErr(errText(e)); } finally { setBusy(false); }
  };
  const have = snap?.accounts.length ?? 0;
  return (
    <Modal title="Restore from a backup" onClose={busy ? () => {} : onClose}
      footer={<><Btn onClick={onClose} disabled={busy}>Cancel</Btn>{info
        ? <Btn kind="danger" disabled={busy} onClick={restore}>{busy ? "Restoring…" : "Replace and restore"}</Btn>
        : <Btn kind="primary" disabled={busy || !path || !pw} onClick={open}>{busy ? "Opening…" : "Open backup"}</Btn>}</>}>
      <div className="row"><Btn onClick={pick} disabled={busy}>Choose file…</Btn><span className="hint ellipsis" title={path}>{path || "No file chosen."}</span></div>
      <Field label="Backup password"><input className="input" type="password" value={pw} onChange={(e) => { setPw(e.target.value); setInfo(null); }} onKeyDown={(e) => e.key === "Enter" && path && pw && !info && open()} /></Field>
      {info && (
        <div className="callout">
          <b>Backup from {new Date(info.created * 1000).toLocaleString()}</b> (RAM {info.app_version}): {info.accounts} account{info.accounts === 1 ? "" : "s"}, {info.cookies} cookie{info.cookies === 1 ? "" : "s"}, {info.passwords} password{info.passwords === 1 ? "" : "s"}.
          <div style={{ marginTop: 6 }}>Restoring <b>replaces</b> the {have} account{have === 1 ? "" : "s"}, the settings and the stored cookies RAM has now. The current data file is kept next to it as <span className="mono">store.before-restore.json</span>.</div>
        </div>
      )}
      {err && <div className="bad">{err}</div>}
    </Modal>
  );
}

/** Shows its children only when cookies can be stored; otherwise offers to
 *  set up the vault (Windows account, one click) or unlock it. */
export function VaultGate({ children, what }: { children: ReactNode; what: string }) {
  const { snap, refresh, toast } = useApp();
  const [unlock, setUnlock] = useState(false);
  const [busy, setBusy] = useState(false);
  const v = snap?.vault;
  if (!v) return null;
  if (!v.configured) {
    return (
      <div className="gate">
        <p style={{ margin: 0 }}><b>Set up the cookie vault first.</b> {what} are encrypted at rest and never written to the plain data file.</p>
        <div className="row" style={{ gap: 8, flexWrap: "wrap" }}>
          <Btn kind="primary" disabled={busy} onClick={async () => { setBusy(true); try { await api.vaultSetupDpapi(); await refresh(); toast("Vault ready (Windows account).", "good"); } catch (e) { toast(errText(e), "bad"); } finally { setBusy(false); } }}>Protect with my Windows account</Btn>
          <span className="hint">Unlocks automatically for your Windows user, unreadable on any other PC. Prefer a master password? Settings → Security.</span>
        </div>
      </div>
    );
  }
  if (!v.unlocked) {
    return (
      <div className="gate">
        <p style={{ margin: 0 }}><b>The vault is locked.</b> Unlock it to store {what.toLowerCase()}.</p>
        <div><Btn kind="primary" onClick={() => setUnlock(true)}>Unlock…</Btn></div>
        {unlock && <VaultUnlockDialog onClose={() => { setUnlock(false); void refresh(); }} />}
      </div>
    );
  }
  return <>{children}</>;
}

type AddTab = "signin" | "cookie" | "many";

/** RoALT-style Add account: sign in with Roblox in your own browser, paste
 *  a cookie, or paste many at once. The username comes from Roblox. */
export function AddAccountDialog({ onClose, onImport }: { onClose: () => void; onImport: () => void }) {
  const { refresh, toast, setSelected, snap } = useApp();
  const [tab, setTab] = useState<AddTab>("signin");
  const [cookie, setCookie] = useState("");
  const [alias, setAlias] = useState("");
  const [many, setMany] = useState("");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState("");
  const [waiting, setWaiting] = useState<string | null>(null); // browser name while the sign-in window is open
  const [progress, setProgress] = useState<{ done: number; total: number } | null>(null);
  const [result, setResult] = useState<import("./api").ManyResult | null>(null);
  const [savePw, setSavePw] = useState(true);
  const [queue, setQueue] = useState<QueueEvent | null>(null);   // the sign-in queue's latest state
  const [queueLog, setQueueLog] = useState<string[]>([]);
  const browser = snap?.browser_name || "";
  const queueRunning = !!queue && queue.state !== "finished";

  // Follow the username:password queue while it runs.
  useEffect(() => {
    let un: (() => void) | undefined;
    api.onSigninQueue((e) => {
      setQueue(e);
      if (e.state === "done" || e.state === "failed" || e.state === "skipped") setQueueLog((l) => [...l, `${e.username}: ${e.state === "done" ? "signed in" : e.message}`].slice(-40));
      if (e.state === "done" || e.state === "finished") void refresh();
    }).then((u) => { un = u; });
    return () => { un?.(); };
  }, [refresh]);

  // Poll the sign-in window until a session is captured or it is closed.
  useEffect(() => {
    if (!waiting) return;
    let alive = true;
    const t = window.setInterval(async () => {
      try {
        const p = await api.signinPoll();
        if (!alive) return;
        if (p.state === "done") {
          setWaiting(null);
          await refresh();
          setSelected([p.account.id]);
          toast(`${p.added ? "Added" : "Refreshed"} ${p.account.username}.`, "good");
          onClose();
        } else if (p.state === "closed") {
          setWaiting(null);
          setErr("The sign-in window was closed before a sign-in happened.");
        }
      } catch (e) { if (alive) { setWaiting(null); setErr(errText(e)); } }
    }, 1500);
    return () => { alive = false; window.clearInterval(t); };
  }, [waiting, refresh, setSelected, toast, onClose]);

  const startSignIn = async () => {
    setErr("");
    try { const name = await api.signinStart(); setWaiting(name); } catch (e) { setErr(errText(e)); }
  };
  const cancelSignIn = async () => { await api.signinCancel().catch(() => {}); setWaiting(null); };
  const submitCookie = async () => {
    setBusy(true); setErr("");
    try {
      const a = await api.addByCookie(cookie, alias);
      await refresh(); setSelected([a.account.id]);
      toast(`${a.added ? "Added" : "Refreshed"} ${a.account.username}.`, "good");
      onClose();
    } catch (e) { setErr(errText(e)); } finally { setBusy(false); }
  };
  const submitMany = async () => {
    setBusy(true); setErr(""); setResult(null); setProgress({ done: 0, total: many.split(/\r?\n/).filter((l) => l.trim()).length });
    const un = await api.onAddManyProgress(setProgress);
    try {
      setQueueLog([]);
      const r = await api.addMany(many, savePw);
      setResult(r); await refresh();
      if (r.added + r.refreshed > 0) toast(`Import: ${r.added} added, ${r.refreshed} refreshed${r.failed ? `, ${r.failed} failed` : ""}.`, r.failed ? "warn" : "good");
      if (r.queued > 0) setQueue({ index: 0, total: r.queued, username: "", state: "opening", message: "Starting the sign-in queue…", done: 0, failed: 0 });
      if (r.failed === 0) { setMany(""); }
    } catch (e) { setErr(errText(e)); } finally { un(); setBusy(false); setProgress(null); }
  };
  const close = () => { if (waiting) void api.signinCancel().catch(() => {}); onClose(); };

  return (
    <Modal title="Add an account" onClose={close} wide
      footer={<>
        <Btn onClick={close}>{result && result.added + result.refreshed > 0 ? "Done" : "Cancel"}</Btn>
        {tab === "cookie" && <Btn kind="primary" onClick={submitCookie} disabled={busy || cookie.trim().length < 32}>Add</Btn>}
        {tab === "many" && <Btn kind="primary" onClick={submitMany} disabled={busy || queueRunning || !many.trim()}>{busy && progress ? `Importing ${progress.done} of ${progress.total}…` : "Import"}</Btn>}
      </>}>
      <p className="muted" style={{ margin: 0 }}>Cookies are encrypted and stay on this PC. RAM asks Roblox whose session it is, so you never type the username.</p>
      <div className="seg">
        <button type="button" className={tab === "signin" ? "active" : ""} onClick={() => setTab("signin")}>Sign in with Roblox</button>
        <button type="button" className={tab === "cookie" ? "active" : ""} onClick={() => setTab("cookie")}>Paste a cookie</button>
        <button type="button" className={tab === "many" ? "active" : ""} onClick={() => setTab("many")}>Add many at once</button>
      </div>
      <VaultGate what="Cookies">
        {tab === "signin" && (
          <>
            <p className="muted" style={{ margin: 0 }}>{browser ? `A ${browser} window opens on Roblox's sign-in page, in a browser profile of its own. Passkeys and 2-step verification work there. The session is deleted from that profile once RAM has it.` : "No Chromium-based browser (Brave, Chrome, Edge, Vivaldi) is installed, so the sign-in window cannot open. Paste a cookie instead."}</p>
            {waiting ? (
              <div className="row" style={{ gap: 10 }}><span className="dot pulse" /><span>Waiting for you to finish signing in in {waiting}…</span><span className="grow" /><Btn sm onClick={cancelSignIn}>Cancel</Btn></div>
            ) : (
              <div><Btn kind="primary" onClick={startSignIn} disabled={!browser}><ExternalLink />Open Roblox sign-in</Btn></div>
            )}
          </>
        )}
        {tab === "cookie" && (
          <>
            <Field label=".ROBLOSECURITY cookie" hint="The raw value or .ROBLOSECURITY=… both work. Anyone with this cookie can sign in as you; never paste it anywhere else.">
              <input className="input mono" type="password" autoFocus value={cookie} onChange={(e) => setCookie(e.target.value)} onKeyDown={(e) => e.key === "Enter" && submitCookie()} placeholder="_|WARNING:-DO-NOT-SHARE-THIS…" />
            </Field>
            <Field label="Alias (optional)"><input className="input" value={alias} onChange={(e) => setAlias(e.target.value)} onKeyDown={(e) => e.key === "Enter" && submitCookie()} placeholder="A name only you see" /></Field>
          </>
        )}
        {tab === "many" && (
          <>
            <Field label="Cookies or username:password, one per line" hint={`Cookies are checked with Roblox and added right away (user:pass:cookie lines keep the cookie). username:password lines are queued: ${browser || "your browser"} opens Roblox's sign-in page for one account at a time, filled in and submitted; solve any check there and RAM saves the session, then moves to the next.`}>
              <textarea className="textarea mono" rows={6} value={many} onChange={(e) => setMany(e.target.value)} placeholder={"username:password\nusername:password\n_|WARNING:-DO-NOT-SHARE-THIS…"} spellCheck={false} disabled={queueRunning} />
            </Field>
            <Toggle checked={savePw} onChange={setSavePw} label="Save passwords in the vault" hint="Encrypted like the cookies, so you can copy a password or user:pass later (right-click → Copy). Off: a password only fills in the sign-in form and is forgotten." />
            {queue && (
              <div className="gate" style={{ background: "var(--accent-wash)", borderColor: "color-mix(in srgb, var(--accent) 45%, var(--border))" }}>
                {queueRunning ? (
                  <>
                    <div className="row" style={{ gap: 10 }}>
                      <span className="dot pulse" />
                      <span className="grow"><b>Signing in {Math.max(1, queue.index)} of {queue.total}{queue.username ? `: ${queue.username}` : ""}</b><br /><span className="muted">{queue.message}</span></span>
                      <Btn sm onClick={() => api.signinQueueSkip().catch(() => {})} title="Leave this account and go to the next">Skip</Btn>
                      <Btn sm kind="danger" onClick={() => api.signinQueueStop().catch(() => {})}>Stop</Btn>
                    </div>
                    <div className="hint">Closing the browser window also stops the queue. Accounts already signed in stay.</div>
                  </>
                ) : (
                  <div><b>Queue finished:</b> {queue.done} signed in{queue.failed ? `, ${queue.failed} not` : ""}.</div>
                )}
                {queueLog.length > 0 && <ul className="hint" style={{ margin: 0, paddingLeft: 18, maxHeight: 96, overflow: "auto" }}>{queueLog.map((l, i) => <li key={i}>{l}</li>)}</ul>}
              </div>
            )}
            <div className="row" style={{ gap: 8, flexWrap: "wrap" }}>
              <Btn onClick={() => { onClose(); onImport(); }}><Download />Import from Roblox Account Manager…</Btn>
              <span className="hint">Its AccountData.json, with its cookies moved into the vault.</span>
            </div>
            {result && (
              <div className="hint">
                {result.added} added, {result.refreshed} refreshed, {result.repeated} repeated, {result.failed} failed{result.queued ? `, ${result.queued} queued for sign-in` : ""}.
                {result.problems.length > 0 && <ul style={{ margin: "4px 0 0", paddingLeft: 18 }}>{result.problems.slice(0, 12).map((p, i) => <li key={i}>{p}</li>)}{result.problems.length > 12 && <li>…and {result.problems.length - 12} more</li>}</ul>}
              </div>
            )}
          </>
        )}
      </VaultGate>
      {err && <div className="bad">{err}</div>}
    </Modal>
  );
}

export function RevealCookieDialog({ id, name, needPassword, onClose, what = "cookie" }: { id: number; name: string; needPassword: boolean; onClose: () => void; what?: "cookie" | "password" | "userpass" }) {
  const { toast } = useApp();
  const [password, setPassword] = useState("");
  const [cookie, setCookie] = useState<string | null>(null);
  const [err, setErr] = useState("");
  const [askPw, setAskPw] = useState(needPassword);
  useEffect(() => {
    if (!needPassword) { void reveal(undefined); }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  const reveal = async (pw?: string) => {
    try { setCookie(await api.cookieReveal(id, pw, what)); setErr(""); }
    catch (e) { const t = errText(e); if (t === "PASSWORD_REQUIRED") setAskPw(true); else setErr(t); }
  };
  const copy = async () => { if (cookie) { await writeText(cookie); toast("Copied. Clear your clipboard when done.", "warn"); onClose(); } };
  return (
    <Modal title={`${what === "cookie" ? "Cookie" : what === "password" ? "Password" : "username:password"} — ${name}`} onClose={onClose} footer={<><Btn onClick={onClose}>Close</Btn>{cookie && <Btn kind="primary" onClick={copy}>Copy</Btn>}</>}>
      {askPw && !cookie && (
        <Field label="Master password"><input className="input" type="password" autoFocus value={password} onChange={(e) => setPassword(e.target.value)} onKeyDown={(e) => e.key === "Enter" && reveal(password)} /></Field>
      )}
      {askPw && !cookie && <Btn kind="primary" onClick={() => reveal(password)}>Confirm</Btn>}
      {cookie && <>
        <p className="warn" style={{ margin: 0 }}>Anyone with this value can sign in as this account.</p>
        <textarea className="textarea mono" readOnly rows={4} value={cookie} onFocus={(e) => e.currentTarget.select()} />
      </>}
      {err && <div className="bad">{err}</div>}
    </Modal>
  );
}

export function VaultSetupDialog({ onClose }: { onClose: () => void }) {
  const { refresh, toast } = useApp();
  const [mode, setMode] = useState<"dpapi" | "password">("dpapi");
  const [pw, setPw] = useState(""); const [pw2, setPw2] = useState("");
  const [recovery, setRecovery] = useState<string | null>(null);
  const [err, setErr] = useState("");
  const submit = async () => {
    try {
      if (mode === "dpapi") { await api.vaultSetupDpapi(); toast("Vault ready (Windows account).", "good"); await refresh(); onClose(); return; }
      if (pw !== pw2) { setErr("The passwords do not match."); return; }
      const code = await api.vaultSetupPassword(pw);
      setRecovery(code); await refresh();
    } catch (e) { setErr(errText(e)); }
  };
  if (recovery) return (
    <Modal title="Recovery code" onClose={onClose} footer={<Btn kind="primary" onClick={async () => { await writeText(recovery); onClose(); }}>Copy and close</Btn>}>
      <p style={{ margin: 0 }}>Save this somewhere safe. It is the <b>only</b> way to unlock the vault if you forget the master password, and it is shown once.</p>
      <div className="mono" style={{ fontSize: 18, letterSpacing: ".04em", padding: "10px 12px", background: "var(--surface-2)", borderRadius: "var(--radius-sm)", userSelect: "text" }}>{recovery}</div>
    </Modal>
  );
  return (
    <Modal title="Set up cookie vault" onClose={onClose} footer={<><Btn onClick={onClose}>Cancel</Btn><Btn kind="primary" onClick={submit} disabled={mode === "password" && pw.length < 8}>Set up</Btn></>}>
      <div className="row" style={{ gap: 6 }}>
        <Btn kind={mode === "dpapi" ? "primary" : undefined} onClick={() => setMode("dpapi")}>Windows account</Btn>
        <Btn kind={mode === "password" ? "primary" : undefined} onClick={() => setMode("password")}>Master password</Btn>
      </div>
      {mode === "dpapi" ? (
        <p className="muted" style={{ margin: 0 }}>Protected with Windows data protection (DPAPI): unlocks automatically for your Windows user and is unreadable on any other PC or account. Reveal/copy still ask for Windows Hello when available.</p>
      ) : <>
        <p className="muted" style={{ margin: 0 }}>Argon2id + AES‑256‑GCM. The vault locks after 15 minutes idle and needs the password to unlock. You get a one‑time recovery code.</p>
        <Field label="Master password"><input className="input" type="password" autoFocus value={pw} onChange={(e) => setPw(e.target.value)} /></Field>
        <Field label="Confirm"><input className="input" type="password" value={pw2} onChange={(e) => setPw2(e.target.value)} onKeyDown={(e) => e.key === "Enter" && submit()} /></Field>
      </>}
      {err && <div className="bad">{err}</div>}
    </Modal>
  );
}

export function VaultUnlockDialog({ onClose }: { onClose: () => void }) {
  const { snap, toast } = useApp();
  const [pw, setPw] = useState(""); const [code, setCode] = useState(""); const [useCode, setUseCode] = useState(false);
  const [err, setErr] = useState("");
  const submit = async () => {
    try { if (useCode) await api.vaultUnlockRecovery(code); else await api.vaultUnlock(pw); toast("Vault unlocked.", "good"); onClose(); }
    catch (e) { setErr(errText(e)); }
  };
  if (snap?.vault.mode === "dpapi") {
    // DPAPI vaults unlock without a password.
    return (
      <Modal title="Unlock vault" onClose={onClose} footer={<><Btn onClick={onClose}>Cancel</Btn><Btn kind="primary" onClick={submit}>Unlock</Btn></>}>
        <p className="muted" style={{ margin: 0 }}>This vault is protected by your Windows account.</p>
        {err && <div className="bad">{err}</div>}
      </Modal>
    );
  }
  return (
    <Modal title="Unlock vault" onClose={onClose} footer={<><Btn onClick={onClose}>Cancel</Btn><Btn kind="primary" onClick={submit}>Unlock</Btn></>}>
      {!useCode ? (
        <Field label="Master password"><input className="input" type="password" autoFocus value={pw} onChange={(e) => setPw(e.target.value)} onKeyDown={(e) => e.key === "Enter" && submit()} /></Field>
      ) : (
        <Field label="Recovery code"><input className="input mono" autoFocus value={code} onChange={(e) => setCode(e.target.value)} onKeyDown={(e) => e.key === "Enter" && submit()} placeholder="XXXXXXXX-XXXXXXXX-XXXXXXXX" /></Field>
      )}
      <Btn kind="ghost" sm onClick={() => setUseCode((v) => !v)}>{useCode ? "Use the password instead" : "Forgot it? Use the recovery code"}</Btn>
      {err && <div className="bad">{err}</div>}
    </Modal>
  );
}

export function ChangePasswordDialog({ onClose }: { onClose: () => void }) {
  const { refresh, toast } = useApp();
  const [cur, setCur] = useState(""); const [pw, setPw] = useState(""); const [pw2, setPw2] = useState("");
  const [recovery, setRecovery] = useState<string | null>(null); const [err, setErr] = useState("");
  const submit = async () => {
    if (pw !== pw2) { setErr("The passwords do not match."); return; }
    try { setRecovery(await api.vaultChangePassword(cur, pw)); await refresh(); toast("Master password changed.", "good"); } catch (e) { setErr(errText(e)); }
  };
  if (recovery) return (
    <Modal title="New recovery code" onClose={onClose} footer={<Btn kind="primary" onClick={async () => { await writeText(recovery); onClose(); }}>Copy and close</Btn>}>
      <p style={{ margin: 0 }}>The old recovery code no longer works. Save this one.</p>
      <div className="mono" style={{ fontSize: 18, padding: "10px 12px", background: "var(--surface-2)", borderRadius: "var(--radius-sm)", userSelect: "text" }}>{recovery}</div>
    </Modal>
  );
  return (
    <Modal title="Change master password" onClose={onClose} footer={<><Btn onClick={onClose}>Cancel</Btn><Btn kind="primary" onClick={submit} disabled={pw.length < 8}>Change</Btn></>}>
      <Field label="Current password"><input className="input" type="password" autoFocus value={cur} onChange={(e) => setCur(e.target.value)} /></Field>
      <Field label="New password"><input className="input" type="password" value={pw} onChange={(e) => setPw(e.target.value)} /></Field>
      <Field label="Confirm"><input className="input" type="password" value={pw2} onChange={(e) => setPw2(e.target.value)} onKeyDown={(e) => e.key === "Enter" && submit()} /></Field>
      {err && <div className="bad">{err}</div>}
    </Modal>
  );
}

export function ImportDialog({ onClose }: { onClose: () => void }) {
  const { snap, refresh, toast } = useApp();
  const [plan, setPlan] = useState<PlanView | null>(null);
  const [sel, setSel] = useState<Set<number>>(new Set());
  const [withCookies, setWithCookies] = useState(true);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState("");
  const [found, setFound] = useState<FoundFile[] | null>(null);
  const [searching, setSearching] = useState(false);
  const [pending, setPending] = useState<string | null>(null);   // a password-protected file waiting for its password
  const [filePw, setFilePw] = useState("");
  const [usedPw, setUsedPw] = useState<string | undefined>(undefined);

  const scan = async (path: string, password?: string) => {
    setBusy(true); setErr("");
    try {
      const p = await api.importScan(path, password);
      if (p.needs_password || p.wrong_password) {
        setPending(path); setPlan(null);
        if (p.wrong_password) setErr("That password does not open the file.");
        return;
      }
      setPending(null); setUsedPw(password); setFilePw("");
      setPlan(p);
      setSel(new Set(p.items.filter((i) => i.selectable && i.selected).map((i) => i.index)));
    } catch (e) { setErr(errText(e)); } finally { setBusy(false); }
  };
  const search = async () => {
    setSearching(true); setErr("");
    try { setFound(await api.importFind()); } catch (e) { setErr(errText(e)); } finally { setSearching(false); }
  };
  // Look for the file as soon as the dialog opens: most people do not know where it is.
  useEffect(() => { void search(); }, []);
  const pickFolder = async () => {
    const dir = await pickDialog({ directory: true, title: "Choose the Roblox Account Manager folder" });
    if (dir && !Array.isArray(dir)) void scan(dir);
  };
  const pickFile = async () => {
    const f = await pickDialog({ title: "Choose AccountData.json or its backup", filters: [{ name: "Account data", extensions: ["json", "backup", "bak", "old"] }, { name: "All files", extensions: ["*"] }] });
    if (f && !Array.isArray(f)) void scan(f);
  };
  const apply = async () => {
    if (!plan) return;
    setBusy(true);
    try {
      const msg = await api.importApply(plan.folder, [...sel], withCookies && plan.cookies_available + plan.passwords_available > 0, usedPw);
      await refresh(); toast(msg, "good"); onClose();
    } catch (e) { setErr(errText(e)); await refresh(); } finally { setBusy(false); }
  };
  const vaultOk = !!snap?.vault.unlocked;
  const secrets = plan ? plan.cookies_available + plan.passwords_available : 0;
  // Cookies can be (re)imported for accounts that are already present, so
  // the button stays live when nothing is selected but cookies are wanted.
  const cookiesOnly = !!plan && sel.size === 0 && withCookies && vaultOk && secrets > 0;
  const kindLabel = (k: string) => k === "PasswordProtected" ? "password-protected" : k === "Dpapi" ? "Windows-protected" : k === "Plaintext" ? "not encrypted" : k.toLowerCase();
  const whenOf = (m: number | null) => m ? new Date(m * 1000).toLocaleDateString() : "";
  return (
    <Modal title="Import from Roblox Account Manager" wide onClose={onClose}
      footer={<><Btn onClick={onClose}>Cancel</Btn>{plan && <Btn onClick={() => { setPlan(null); setErr(""); }}>Back</Btn>}{plan && <Btn kind="primary" onClick={apply} disabled={busy || (sel.size === 0 && !cookiesOnly)}>{cookiesOnly ? "Import cookies" : `Import ${sel.size}`}</Btn>}</>}>
      {pending ? (
        <>
          <p style={{ margin: 0 }}><b>This file is locked with a Roblox Account Manager password.</b></p>
          <div className="hint mono ellipsis" title={pending}>{pending}</div>
          <Field label="The password you set in Roblox Account Manager" hint="Used once to read the file; it is not stored. The file itself is not changed.">
            <input className="input" type="password" autoFocus value={filePw} onChange={(e) => setFilePw(e.target.value)} onKeyDown={(e) => e.key === "Enter" && filePw && scan(pending, filePw)} />
          </Field>
          <div className="row" style={{ gap: 8 }}>
            <Btn kind="primary" disabled={busy || !filePw} onClick={() => scan(pending, filePw)}>{busy ? "Unlocking… (takes a few seconds)" : "Unlock"}</Btn>
            <Btn onClick={() => { setPending(null); setFilePw(""); setErr(""); }}>Back</Btn>
          </div>
        </>
      ) : !plan ? (
        <>
          <p className="muted" style={{ margin: 0 }}>RAM looks for <span className="mono">AccountData.json</span> (and its backup) on this PC. Files are only read; nothing in them is changed, and you see a preview before anything is imported.</p>
          <div style={{ border: "1px solid var(--border)", borderRadius: "var(--radius-sm)", maxHeight: 230, overflow: "auto" }}>
            {searching ? <div className="row" style={{ padding: 12, gap: 10 }}><span className="dot pulse" /><span>Searching Desktop, Downloads, Documents and your user folder…</span></div>
              : !found || found.length === 0 ? <div className="muted" style={{ padding: 12 }}>No AccountData.json found in the usual places. Choose the folder or the file yourself below.</div>
              : found.map((f) => (
                <div key={f.path} className="found" onClick={() => !busy && scan(f.path)} title="Preview this file">
                  <div className="grow" style={{ minWidth: 0 }}>
                    <div className="ellipsis"><b>{f.path.split(/[\\/]/).slice(-2).join("\\")}</b>{f.is_backup && <span className="badge muted" style={{ marginLeft: 8 }}>backup</span>}{f.kind === "PasswordProtected" && <span className="badge warn" style={{ marginLeft: 8 }}>needs its password</span>}</div>
                    <div className="hint mono ellipsis">{f.path}</div>
                  </div>
                  <div className="hint" style={{ textAlign: "right", whiteSpace: "nowrap" }}>{kindLabel(f.kind)}<br />{whenOf(f.modified)}</div>
                </div>
              ))}
          </div>
          <div className="row" style={{ gap: 8, flexWrap: "wrap" }}>
            <Btn onClick={search} disabled={searching || busy}>Search again</Btn>
            <Btn onClick={pickFolder} disabled={busy}>Choose folder…</Btn>
            <Btn onClick={pickFile} disabled={busy}>Choose file…</Btn>
          </div>
        </>
      ) : (
        <>
          <div className="row muted" style={{ fontSize: 12 }}>
            <span><b className="good">{plan.add}</b> to add</span><span><b>{plan.update}</b> to update</span><span>{plan.unchanged} present</span><span>{plan.skip} skipped</span>
            <span className="grow" />
            <span>{plan.cookies_available} cookie{plan.cookies_available === 1 ? "" : "s"}{plan.passwords_available ? `, ${plan.passwords_available} password${plan.passwords_available === 1 ? "" : "s"}` : ""} in file</span>
          </div>
          {secrets > 0 && (
            vaultOk
              ? <Toggle checked={withCookies} onChange={setWithCookies} label={`Also move the cookies${plan.passwords_available ? " and saved passwords" : ""} into the vault`} hint="Accounts already in RAM get theirs too, and every account with a cookie is checked right away." />
              : <VaultGate what="Imported cookies"><span /></VaultGate>
          )}
          <div style={{ maxHeight: 320, overflow: "auto", border: "1px solid var(--border)", borderRadius: "var(--radius-sm)" }}>
            <table className="table">
              <thead><tr><th style={{ width: 32 }}></th><th>Item</th><th>Action</th><th>Details</th></tr></thead>
              <tbody>
                {plan.items.map((it) => (
                  <tr key={it.index} className={sel.has(it.index) ? "selected" : ""} style={{ opacity: it.selectable ? 1 : .55 }}
                    onClick={() => { if (!it.selectable) return; setSel((s) => { const n = new Set(s); n.has(it.index) ? n.delete(it.index) : n.add(it.index); return n; }); }}>
                    <td><input type="checkbox" readOnly disabled={!it.selectable} checked={sel.has(it.index)} /></td>
                    <td><span className="muted">{it.kind}:</span> {it.label}</td>
                    <td>{it.action}</td>
                    <td className="muted">{it.detail}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          {plan.notices.length > 0 && <ul className="muted" style={{ margin: 0, paddingLeft: 18, fontSize: 12 }}>{plan.notices.map((n, i) => <li key={i}>{n}</li>)}</ul>}
        </>
      )}
      {err && <div className="bad">{err}</div>}
    </Modal>
  );
}
