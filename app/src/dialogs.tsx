// Modal flows: add account, set/reveal cookie, vault setup/unlock/change, import, confirm.

import { useEffect, useState, type ReactNode } from "react";
import { open as pickDialog } from "@tauri-apps/plugin-dialog";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { Download, ExternalLink } from "lucide-react";
import { api, type PlanView } from "./api";
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
  const browser = snap?.browser_name || "";

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
      const r = await api.addMany(many);
      setResult(r); await refresh();
      if (r.added + r.refreshed > 0) toast(`Import: ${r.added} added, ${r.refreshed} refreshed${r.failed ? `, ${r.failed} failed` : ""}.`, r.failed ? "warn" : "good");
      if (r.failed === 0) { setMany(""); }
    } catch (e) { setErr(errText(e)); } finally { un(); setBusy(false); setProgress(null); }
  };
  const close = () => { if (waiting) void api.signinCancel().catch(() => {}); onClose(); };

  return (
    <Modal title="Add an account" onClose={close} wide
      footer={<>
        <Btn onClick={close}>{result && result.added + result.refreshed > 0 ? "Done" : "Cancel"}</Btn>
        {tab === "cookie" && <Btn kind="primary" onClick={submitCookie} disabled={busy || cookie.trim().length < 32}>Add</Btn>}
        {tab === "many" && <Btn kind="primary" onClick={submitMany} disabled={busy || !many.trim()}>{busy && progress ? `Importing ${progress.done} of ${progress.total}…` : "Import"}</Btn>}
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
            <Field label="Cookies, one per line" hint="user:pass:cookie lines from another manager keep only the cookie. username:password lines are skipped: RAM never signs in with passwords; use Sign in with Roblox for those.">
              <textarea className="textarea mono" rows={7} value={many} onChange={(e) => setMany(e.target.value)} placeholder={"_|WARNING:-DO-NOT-SHARE-THIS…\n_|WARNING:-DO-NOT-SHARE-THIS…"} spellCheck={false} />
            </Field>
            <div className="row" style={{ gap: 8, flexWrap: "wrap" }}>
              <Btn onClick={() => { onClose(); onImport(); }}><Download />Import from Roblox Account Manager…</Btn>
              <span className="hint">Its AccountData.json, with its cookies moved into the vault.</span>
            </div>
            {result && (
              <div className="hint">
                {result.added} added, {result.refreshed} refreshed, {result.repeated} repeated, {result.failed} failed.
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

export function RevealCookieDialog({ id, name, needPassword, onClose }: { id: number; name: string; needPassword: boolean; onClose: () => void }) {
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
    try { setCookie(await api.cookieReveal(id, pw)); setErr(""); }
    catch (e) { const t = errText(e); if (t === "PASSWORD_REQUIRED") setAskPw(true); else setErr(t); }
  };
  const copy = async () => { if (cookie) { await writeText(cookie); toast("Cookie copied. Clear your clipboard when done.", "warn"); onClose(); } };
  return (
    <Modal title={`Cookie — ${name}`} onClose={onClose} footer={<><Btn onClick={onClose}>Close</Btn>{cookie && <Btn kind="primary" onClick={copy}>Copy</Btn>}</>}>
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
  const pick = async () => {
    const dir = await pickDialog({ directory: true, title: "Choose the Roblox Account Manager folder" });
    if (!dir || Array.isArray(dir)) return;
    setBusy(true);
    try {
      const p = await api.importScan(dir);
      setPlan(p);
      setSel(new Set(p.items.filter((i) => i.selectable && i.selected).map((i) => i.index)));
    } catch (e) { setErr(errText(e)); } finally { setBusy(false); }
  };
  const apply = async () => {
    if (!plan) return;
    setBusy(true);
    try {
      const msg = await api.importApply(plan.folder, [...sel], withCookies && plan.cookies_available > 0);
      await refresh(); toast(msg, "good"); onClose();
    } catch (e) { setErr(errText(e)); await refresh(); } finally { setBusy(false); }
  };
  const vaultOk = !!snap?.vault.unlocked;
  // Cookies can be (re)imported for accounts that are already present, so
  // the button stays live when nothing is selected but cookies are wanted.
  const cookiesOnly = !!plan && sel.size === 0 && withCookies && vaultOk && plan.cookies_available > 0;
  return (
    <Modal title="Import from Roblox Account Manager" wide onClose={onClose}
      footer={<><Btn onClick={onClose}>Cancel</Btn>{plan && <Btn kind="primary" onClick={apply} disabled={busy || (sel.size === 0 && !cookiesOnly)}>{cookiesOnly ? "Import cookies" : `Import ${sel.size}`}</Btn>}</>}>
      {!plan ? (
        <>
          <p className="muted" style={{ margin: 0 }}>Pick the folder that contains <span className="mono">AccountData.json</span>. The folder is only read; nothing in it is changed. You'll see a preview before anything is imported.</p>
          <Btn kind="primary" onClick={pick} disabled={busy}>Choose folder…</Btn>
        </>
      ) : (
        <>
          <div className="row muted" style={{ fontSize: 12 }}>
            <span><b className="good">{plan.add}</b> to add</span><span><b>{plan.update}</b> to update</span><span>{plan.unchanged} present</span><span>{plan.skip} skipped</span>
            <span className="grow" />
            <span>{plan.cookies_available} cookie{plan.cookies_available === 1 ? "" : "s"} in file</span>
          </div>
          {plan.cookies_available > 0 && (
            vaultOk
              ? <Toggle checked={withCookies} onChange={setWithCookies} label={`Also move the ${plan.cookies_available} cookie${plan.cookies_available === 1 ? "" : "s"} into the vault`} hint="Accounts already in RAM get their cookie too, and every account with a cookie is checked right away." />
              : <VaultGate what="Imported cookies"><span /></VaultGate>
          )}
          <div style={{ maxHeight: 340, overflow: "auto", border: "1px solid var(--border)", borderRadius: "var(--radius-sm)" }}>
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
