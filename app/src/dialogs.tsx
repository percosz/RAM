// Modal flows: add account, set/reveal cookie, vault setup/unlock/change, import, confirm.

import { useEffect, useState } from "react";
import { open as pickDialog } from "@tauri-apps/plugin-dialog";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
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

export function AddAccountDialog({ onClose }: { onClose: () => void }) {
  const { refresh, toast, setSelected } = useApp();
  const [username, setUsername] = useState("");
  const [alias, setAlias] = useState("");
  const [err, setErr] = useState("");
  const submit = async () => {
    try {
      const a = await api.addAccount(username, alias);
      await refresh();
      setSelected([a.id]);
      toast(`Added ${a.username}. Add a cookie, or open the browser and sign in.`, "good");
      onClose();
    } catch (e) { setErr(errText(e)); }
  };
  return (
    <Modal title="Add account" onClose={onClose} footer={<><Btn onClick={onClose}>Cancel</Btn><Btn kind="primary" onClick={submit} disabled={username.trim().length < 3}>Add</Btn></>}>
      <p className="muted" style={{ margin: 0 }}>RAM stores the name now. Sign in through <b>Open Browser</b> and capture the session, or paste a cookie.</p>
      <Field label="Roblox username"><input className="input" autoFocus value={username} onChange={(e) => setUsername(e.target.value)} onKeyDown={(e) => e.key === "Enter" && submit()} placeholder="Exactly as it appears on Roblox" /></Field>
      <Field label="Alias (optional)"><input className="input" value={alias} onChange={(e) => setAlias(e.target.value)} onKeyDown={(e) => e.key === "Enter" && submit()} placeholder="A name only you see" /></Field>
      {err && <div className="bad">{err}</div>}
    </Modal>
  );
}

export function SetCookieDialog({ id, name, onClose }: { id: number; name: string; onClose: () => void }) {
  const { toast } = useApp();
  const [cookie, setCookie] = useState("");
  const [err, setErr] = useState("");
  const submit = async () => {
    try { await api.cookieSet(id, cookie); toast(`Cookie stored for ${name}. Checking…`, "good"); onClose(); } catch (e) { setErr(errText(e)); }
  };
  return (
    <Modal title={`Store cookie — ${name}`} onClose={onClose} footer={<><Btn onClick={onClose}>Cancel</Btn><Btn kind="primary" onClick={submit} disabled={cookie.trim().length < 32}>Store</Btn></>}>
      <p className="muted" style={{ margin: 0 }}>Paste the account's <span className="mono">.ROBLOSECURITY</span> cookie. It is encrypted in the vault and never written to the plain data file or the log.</p>
      <Field label=".ROBLOSECURITY"><input className="input mono" type="password" autoFocus value={cookie} onChange={(e) => setCookie(e.target.value)} onKeyDown={(e) => e.key === "Enter" && submit()} placeholder="_|WARNING:-DO-NOT-SHARE-THIS…" /></Field>
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
  return (
    <Modal title="Import from Roblox Account Manager" wide onClose={onClose}
      footer={<><Btn onClick={onClose}>Cancel</Btn>{plan && <Btn kind="primary" onClick={apply} disabled={busy || sel.size === 0}>Import {sel.size}</Btn>}</>}>
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
            <Toggle checked={withCookies && vaultOk} onChange={setWithCookies} label="Also move the cookies into the vault" hint={vaultOk ? "Imported accounts will be checked right away." : "Unlock (or set up) the vault first to import cookies."} />
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
