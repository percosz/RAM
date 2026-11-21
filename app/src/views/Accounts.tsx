// The main screen: toolbar, account list, details pane, launch bar.

import { useEffect, useMemo, useRef, useState } from "react";
import { Plus, Download, RefreshCw, Trash2, Globe, KeyRound, Eye, Star, Crown, Play, UserPlus, Search, ScanLine, PencilLine } from "lucide-react";
import { api, ago, signInLabel, type Account } from "../api";
import { Avatar, Btn, Field, Modal, StatusBadge, Toggle, tone } from "../components";
import { useApp } from "../store";
import { AddAccountDialog, ConfirmDialog, ImportDialog, RevealCookieDialog, SetCookieDialog } from "../dialogs";

const ROGUE_LINEAGE = 3016661674;

function matches(a: Account, q: string): boolean {
  if (!q) return true;
  const s = q.toLowerCase();
  return [a.username, a.alias, a.display_name, a.group, a.notes, ...a.tags].some((v) => v.toLowerCase().includes(s));
}

export function AccountsView({ onNeedUnlock }: { onNeedUnlock: () => void }) {
  const { snap, settings, accounts, selected, setSelected, toast, refresh, launchStatus } = useApp();
  const classic = settings?.layout === "Classic";
  const [q, setQ] = useState("");
  const [group, setGroup] = useState("");
  const [dialog, setDialog] = useState<null | { kind: "add" } | { kind: "import" } | { kind: "cookie"; id: number } | { kind: "reveal"; id: number } | { kind: "remove"; ids: number[] } | { kind: "removeCookie"; id: number } | { kind: "details"; id: number }>(null);
  const [placeId, setPlaceId] = useState(String(ROGUE_LINEAGE));
  const [jobId, setJobId] = useState("");
  const [follow, setFollow] = useState("");
  const anchor = useRef<number | null>(null);

  const groups = useMemo(() => Array.from(new Set(accounts.map((a) => a.group).filter(Boolean))).sort((x, y) => x.localeCompare(y)), [accounts]);
  const visible = useMemo(() => accounts.filter((a) => matches(a, q) && (!group || a.group === group)), [accounts, q, group]);
  const grouped = useMemo(() => {
    const m = new Map<string, Account[]>();
    for (const a of visible) { const k = a.group || ""; if (!m.has(k)) m.set(k, []); m.get(k)!.push(a); }
    return [...m.entries()].sort(([a], [b]) => (a === "" ? 1 : b === "" ? -1 : a.localeCompare(b)));
  }, [visible]);
  const one = selected.length === 1 ? accounts.find((a) => a.id === selected[0]) ?? null : null;
  const vaultUnlocked = !!snap?.vault.unlocked;
  const vaultConfigured = !!snap?.vault.configured;

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const el = e.target as HTMLElement;
      if (el.tagName === "INPUT" || el.tagName === "TEXTAREA") return;
      if (e.ctrlKey && e.key.toLowerCase() === "a") { e.preventDefault(); setSelected(visible.filter((a) => !a.is_main).map((a) => a.id)); }
      if (e.key === "Delete" && selected.length) setDialog({ kind: "remove", ids: selected });
      if (e.key === "Enter" && selected.length) void join();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [visible, selected, placeId, jobId]);

  const select = (a: Account, e: React.MouseEvent) => {
    const ids = visible.map((x) => x.id);
    if (e.shiftKey && anchor.current != null) {
      const i = ids.indexOf(anchor.current), j = ids.indexOf(a.id);
      if (i >= 0 && j >= 0) { const [s, t] = i < j ? [i, j] : [j, i]; setSelected(ids.slice(s, t + 1)); return; }
    }
    if (e.ctrlKey) { setSelected(selected.includes(a.id) ? selected.filter((x) => x !== a.id) : [...selected, a.id]); }
    else setSelected([a.id]);
    anchor.current = a.id;
  };

  const needCookieVault = (): boolean => {
    if (!vaultConfigured) { toast("Set up the cookie vault in Settings → Security first.", "warn"); return true; }
    if (!vaultUnlocked) { onNeedUnlock(); return true; }
    return false;
  };

  const join = async () => {
    if (!selected.length) { toast("Select an account to launch.", "warn"); return; }
    const pid = Number(placeId.trim().match(/\d+/)?.[0] ?? 0);
    if (!pid) { toast("Enter a place ID.", "warn"); return; }
    if (needCookieVault()) return;
    try { await api.launch(selected, pid, jobId.trim() || null); } catch (e) { toast(String(e), "bad"); }
  };
  const doFollow = async () => {
    if (!selected.length) { toast("Select an account to launch.", "warn"); return; }
    if (!follow.trim()) { toast("Enter a username to follow.", "warn"); return; }
    if (needCookieVault()) return;
    try { await api.follow(selected, follow); } catch (e) { toast(String(e), "bad"); }
  };
  const openBrowser = async (id: number) => {
    try { await api.openBrowser(id); } catch (e) { toast(String(e), "bad"); }
  };
  const capture = async (id: number) => {
    if (needCookieVault()) return;
    try { await api.browserCapture(id); toast("Session captured. Checking…", "good"); } catch (e) { toast(String(e), "bad"); }
  };
  const check = async (ids: number[]) => {
    if (!ids.length) return;
    if (!vaultUnlocked && vaultConfigured) { onNeedUnlock(); return; }
    try { await api.checkAccounts(ids); } catch (e) { toast(String(e), "bad"); }
  };
  const remove = async (ids: number[]) => {
    try { await api.removeAccounts(ids); setSelected([]); await refresh(); toast(`Removed ${ids.length} account${ids.length === 1 ? "" : "s"}.`); } catch (e) { toast(String(e), "bad"); }
  };

  const detailsProps = (a: Account) => ({
    a,
    onOpenBrowser: () => openBrowser(a.id),
    onCapture: () => capture(a.id),
    onCheck: () => check([a.id]),
    onSetCookie: () => { if (!needCookieVault()) setDialog({ kind: "cookie", id: a.id }); },
    onReveal: () => { if (!needCookieVault()) setDialog({ kind: "reveal", id: a.id }); },
    onRemoveCookie: () => setDialog({ kind: "removeCookie", id: a.id }),
    onRemove: () => setDialog({ kind: "remove", ids: [a.id] }),
  });
  const detailsAccount = dialog?.kind === "details" ? accounts.find((a) => a.id === dialog.id) ?? null : null;

  return (
    <div className={`accounts ${classic ? "classic" : ""}`}>
      <div className="toolbar">
        <div className="row grow" style={{ position: "relative" }}>
          <Search style={{ position: "absolute", left: 10, width: 15, height: 15, color: "var(--muted)", pointerEvents: "none" }} />
          <input className="input grow" style={{ paddingLeft: 32 }} placeholder="Filter accounts…" value={q} onChange={(e) => setQ(e.target.value)} />
        </div>
        {groups.length > 0 && (
          <select className="select" value={group} onChange={(e) => setGroup(e.target.value)}>
            <option value="">All groups ({accounts.length})</option>
            {groups.map((g) => <option key={g} value={g}>{g} ({accounts.filter((a) => a.group === g).length})</option>)}
          </select>
        )}
        <Btn onClick={() => check(selected.length ? selected : accounts.map((a) => a.id))} title="Check sign-in (selected, or all)"><RefreshCw />Check</Btn>
        <Btn onClick={() => setDialog({ kind: "import" })}><Download />Import</Btn>
        <Btn kind="primary" onClick={() => setDialog({ kind: "add" })}><Plus />Add</Btn>
      </div>

      {classic && (
        <div className="strip">
          {one ? (
            <>
              <Avatar name={one.alias || one.display_name || one.username} seed={one.id} />
              <span className="who ellipsis"><b>{one.alias || one.display_name || one.username}</b> <span className="muted">@{one.username}</span></span>
              <Btn sm kind="primary" onClick={() => openBrowser(one.id)}><Globe />Open Browser</Btn>
              <Btn sm onClick={() => capture(one.id)} title="Save the session from the open browser window into the vault"><ScanLine />Capture</Btn>
              <Btn sm onClick={() => check([one.id])} disabled={one.checking}><RefreshCw />Check</Btn>
              <Btn sm onClick={() => { if (!needCookieVault()) setDialog({ kind: "cookie", id: one.id }); }}><KeyRound />Set cookie…</Btn>
              <Btn sm onClick={() => { if (!needCookieVault()) setDialog({ kind: "reveal", id: one.id }); }} disabled={!one.has_cookie}><Eye />Reveal</Btn>
              <Btn sm onClick={() => setDialog({ kind: "details", id: one.id })}><PencilLine />Details…</Btn>
              <span className="grow" />
              <Btn sm kind="danger" onClick={() => setDialog({ kind: "remove", ids: [one.id] })}><Trash2 />Remove</Btn>
            </>
          ) : (
            <span className="muted">{selected.length > 1 ? `${selected.length} accounts selected — Join Server launches them one by one.` : "Select an account for its actions; double-click to join."}</span>
          )}
        </div>
      )}

      <div className="list">
        {accounts.length === 0 ? (
          <div className="empty"><div><b>No accounts yet.</b><br />Add one, or import from Roblox Account Manager.<br /><span className="muted">Then open the browser to sign in, or paste a cookie.</span></div></div>
        ) : visible.length === 0 ? (
          <div className="empty"><div>No accounts match.</div></div>
        ) : grouped.map(([g, list]) => (
          <div key={g || "__none"}>
            {(groups.length > 0) && <div className="group-h">{g || "No group"} <span className="count">{list.length}</span></div>}
            {list.map((a, i) => {
              const ls = launchStatus[a.id];
              const sub = ls && ls.state !== "Running" ? ls.message : a.display_name && a.display_name !== a.username ? `@${a.username}` : a.sign_in.detail && a.sign_in.state !== "Verified" ? a.sign_in.detail : a.username;
              return (
                <div key={a.id} className={`acct ${selected.includes(a.id) ? "selected" : ""}`} onClick={(e) => select(a, e)} onDoubleClick={() => { setSelected([a.id]); void join(); }}>
                  <Avatar name={a.alias || a.display_name || a.username} seed={i + a.id} />
                  <div className="grow" style={{ minWidth: 0 }}>
                    <div className="name ellipsis">{a.alias || a.display_name || a.username}{a.is_main && <span className="main-tag">MAIN</span>}{a.favorite && <Star style={{ width: 12, height: 12, marginLeft: 6, color: "var(--warn)", verticalAlign: "-1px" }} />}</div>
                    <div className={`sub ellipsis ${ls?.state === "Failed" ? "bad" : ""}`}>{sub}</div>
                  </div>
                  <div><StatusBadge a={a} /></div>
                  <div className="right">{a.client === "Running" ? <span className="accent">● in game</span> : a.robux != null ? <span>R$ {a.robux.toLocaleString()}</span> : a.has_cookie ? <span title="Cookie stored"><KeyRound style={{ width: 13, height: 13 }} /></span> : ""}</div>
                  <div className="right">{ago(a.last_used)}</div>
                  <div className="right">{a.premium ? <Crown style={{ width: 14, height: 14, color: "var(--warn)" }} /> : null}</div>
                </div>
              );
            })}
          </div>
        ))}
      </div>

      {!classic && (
        <aside className="details">
          {one ? <Details {...detailsProps(one)} /> : (
            <div className="empty"><div>{selected.length > 1 ? <><b>{selected.length} accounts selected</b><br /><span className="muted">Join Server launches them one by one{snap?.settings.launch_spacing_secs ? `, ${snap.settings.launch_spacing_secs}s apart` : ""}.</span></> : <>Select an account to see its details.</>}</div></div>
          )}
        </aside>
      )}

      <div className="launchbar">
        <div className="lgroup">
          <input className="input place mono" placeholder="Place ID" value={placeId} onChange={(e) => setPlaceId(e.target.value)} onKeyDown={(e) => e.key === "Enter" && join()} title="Place ID or game link" />
          <input className="input job mono" placeholder="Job ID (optional)" value={jobId} onChange={(e) => setJobId(e.target.value)} onKeyDown={(e) => e.key === "Enter" && join()} />
          <Btn kind="primary" onClick={join} disabled={!selected.length} title={selected.length ? `Launch ${selected.length} selected` : "Select accounts to launch"}><Play />Join Server</Btn>
        </div>
        <div className="lgroup">
          <input className="input user" placeholder="Username" value={follow} onChange={(e) => setFollow(e.target.value)} onKeyDown={(e) => e.key === "Enter" && doFollow()} />
          <Btn onClick={doFollow} disabled={!selected.length} title="Join the game this user is in"><UserPlus />Follow</Btn>
        </div>
      </div>

      {detailsAccount && (
        <Modal title={detailsAccount.username} onClose={() => setDialog(null)}>
          <div className="details modal-details"><Details {...detailsProps(detailsAccount)} /></div>
        </Modal>
      )}
      {dialog?.kind === "add" && <AddAccountDialog onClose={() => setDialog(null)} />}
      {dialog?.kind === "import" && <ImportDialog onClose={() => setDialog(null)} />}
      {dialog?.kind === "cookie" && <SetCookieDialog id={dialog.id} name={accounts.find((a) => a.id === dialog.id)?.username ?? ""} onClose={() => setDialog(null)} />}
      {dialog?.kind === "reveal" && <RevealCookieDialog id={dialog.id} name={accounts.find((a) => a.id === dialog.id)?.username ?? ""} needPassword={snap?.vault.mode === "password" && !snap.vault.hello_available} onClose={() => setDialog(null)} />}
      {dialog?.kind === "remove" && <ConfirmDialog danger title="Remove account" text={`Remove ${dialog.ids.length === 1 ? accounts.find((a) => a.id === dialog.ids[0])?.username : dialog.ids.length + " accounts"} from RAM? Any stored cookie is deleted from this PC; the Roblox account itself is not affected.`} onYes={() => remove(dialog.ids)} onClose={() => setDialog(null)} />}
      {dialog?.kind === "removeCookie" && <ConfirmDialog danger title="Remove cookie" text="Delete the stored cookie for this account? The account stays in the list." onYes={async () => { await api.cookieRemove(dialog.id); await refresh(); }} onClose={() => setDialog(null)} />}
    </div>
  );
}

function Details({ a, onOpenBrowser, onCapture, onCheck, onSetCookie, onReveal, onRemoveCookie, onRemove }: {
  a: Account; onOpenBrowser: () => void; onCapture: () => void; onCheck: () => void; onSetCookie: () => void; onReveal: () => void; onRemoveCookie: () => void; onRemove: () => void;
}) {
  const { refresh, toast } = useApp();
  const [alias, setAlias] = useState(a.alias);
  const [notes, setNotes] = useState(a.notes);
  const [grp, setGrp] = useState(a.group);
  useEffect(() => { setAlias(a.alias); setNotes(a.notes); setGrp(a.group); }, [a.id, a.alias, a.notes, a.group]);
  const save = async (patch: Parameters<typeof api.updateAccount>[0]) => { try { await api.updateAccount(patch); await refresh(); } catch (e) { toast(String(e), "bad"); } };
  const t = tone(a.sign_in.state);
  return (
    <>
      <div className="head">
        <Avatar lg name={a.alias || a.display_name || a.username} seed={a.id} />
        <div className="t grow">
          <b className="ellipsis">{a.display_name || a.alias || a.username}</b>
          <span className="ellipsis">@{a.username}{a.roblox_user_id ? ` · ID ${a.roblox_user_id}` : ""}</span>
        </div>
      </div>
      <div className="chips">
        <span className={`badge ${t}`} title={a.sign_in.detail}><span className="dot" />{a.checking ? "Checking…" : signInLabel(a.sign_in.state)}{a.sign_in.last_check ? ` · ${ago(a.sign_in.last_check)}` : ""}</span>
        {a.robux != null && <span className="badge accent">R$ {a.robux.toLocaleString()}</span>}
        {a.premium && <span className="badge warn">Premium</span>}
        {a.client === "Running" && <span className="badge good">In game</span>}
        {a.has_cookie ? <span className="badge muted"><KeyRound style={{ width: 11, height: 11 }} />cookie</span> : <span className="badge muted">no cookie</span>}
      </div>
      {a.sign_in.detail && a.sign_in.state !== "Verified" && <div className="hint">{a.sign_in.detail}</div>}

      <div className="actions">
        <Btn kind="primary" onClick={onOpenBrowser} title="Open roblox.com signed in as this account (or sign in there)"><Globe />Open Browser</Btn>
        <Btn onClick={onCapture} title="Save the session from the open browser window into the vault"><ScanLine />Capture session</Btn>
        <Btn onClick={onCheck} disabled={a.checking}><RefreshCw />Check</Btn>
        <Btn onClick={onSetCookie}><KeyRound />Set cookie…</Btn>
        <Btn onClick={onReveal} disabled={!a.has_cookie}><Eye />Reveal cookie</Btn>
        <Btn kind="danger" onClick={onRemoveCookie} disabled={!a.has_cookie}><Trash2 />Remove cookie</Btn>
      </div>

      <Field label="Alias">
        <div className="inline"><input className="input" value={alias} onChange={(e) => setAlias(e.target.value)} onKeyDown={(e) => e.key === "Enter" && save({ id: a.id, alias })} placeholder="A name only you see" /><Btn sm onClick={() => save({ id: a.id, alias })} disabled={alias === a.alias}>Set</Btn></div>
      </Field>
      <Field label="Group">
        <div className="inline"><input className="input" value={grp} onChange={(e) => setGrp(e.target.value)} onKeyDown={(e) => e.key === "Enter" && save({ id: a.id, group: grp })} placeholder="e.g. Farm" /><Btn sm onClick={() => save({ id: a.id, group: grp })} disabled={grp === a.group}>Set</Btn></div>
      </Field>
      <Field label="Description">
        <textarea className="textarea" rows={4} value={notes} onChange={(e) => setNotes(e.target.value)} placeholder="Notes…" />
        <div><Btn sm onClick={() => save({ id: a.id, notes })} disabled={notes === a.notes}>Set description</Btn></div>
      </Field>
      <Toggle checked={a.is_main} onChange={async (v) => { await api.setMain(v ? a.id : null); await refresh(); }} label="Main account" hint="Sorted first and left out of Select All." />
      <Toggle checked={a.favorite} onChange={(v) => save({ id: a.id, favorite: v })} label="Favorite" />
      <div><Btn kind="danger" sm onClick={onRemove}><Trash2 />Remove account</Btn></div>
    </>
  );
}
