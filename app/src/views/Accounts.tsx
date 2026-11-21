// The main screen: toolbar, account list, details pane, launch bar.

import { useEffect, useMemo, useRef, useState } from "react";
import { Plus, Download, RefreshCw, Trash2, Globe, KeyRound, Eye, Star, Crown, Play, UserPlus, Search, ScanLine, Gauge } from "lucide-react";
import { api, ago, signInLabel, type Account, type Game } from "../api";
import { Avatar, Btn, Field, StatusBadge, Toggle, tone } from "../components";
import { useApp } from "../store";
import { AddAccountDialog, ConfirmDialog, ImportDialog, RevealCookieDialog, SetCookieDialog } from "../dialogs";

const ROGUE_LINEAGE = 3016661674;

function matches(a: Account, q: string): boolean {
  if (!q) return true;
  const s = q.toLowerCase();
  return [a.username, a.alias, a.display_name, a.group, a.notes, ...a.tags].some((v) => v.toLowerCase().includes(s));
}

/** Second line of a row: launch progress, else alias and description, else the sign-in detail. */
function subtitle(a: Account, ls?: { state: string; message: string }): string {
  if (ls && ls.state !== "Running" && ls.message) return ls.message;
  const parts = [a.alias, a.notes.split(/\r?\n/)[0] ?? ""].map((s) => s.trim()).filter(Boolean);
  if (parts.length) return parts.join(" · ");
  if (a.display_name && a.display_name !== a.username) return a.display_name;
  if (a.sign_in.detail && a.sign_in.state !== "Verified") return a.sign_in.detail;
  return "";
}

/** Place-name lookup for the launch bar, debounced; the cache lives in the store. */
function usePlaceInfo(placeId: string): { state: "idle" | "loading" | "ok" | "unknown" | "error"; game: Game | null; error: string } {
  const [r, setR] = useState<{ state: "idle" | "loading" | "ok" | "unknown" | "error"; game: Game | null; error: string }>({ state: "idle", game: null, error: "" });
  useEffect(() => {
    const pid = Number(placeId.trim().match(/\d{3,}/)?.[0] ?? 0);
    if (!pid) { setR({ state: "idle", game: null, error: "" }); return; }
    let alive = true;
    setR((p) => ({ ...p, state: "loading" }));
    const t = window.setTimeout(async () => {
      try {
        const g = await api.placeInfo(pid);
        if (alive) setR(g ? { state: "ok", game: g, error: "" } : { state: "unknown", game: null, error: "" });
      } catch (e) {
        if (alive) setR({ state: "error", game: null, error: String(e) });
      }
    }, 450);
    return () => { alive = false; window.clearTimeout(t); };
  }, [placeId]);
  return r;
}

export function AccountsView({ onNeedUnlock }: { onNeedUnlock: () => void }) {
  const { snap, accounts, selected, setSelected, toast, refresh, launchStatus } = useApp();
  const [q, setQ] = useState("");
  const [group, setGroup] = useState("");
  const [dialog, setDialog] = useState<null | { kind: "add" } | { kind: "import" } | { kind: "cookie"; id: number } | { kind: "reveal"; id: number } | { kind: "remove"; ids: number[] } | { kind: "removeCookie"; id: number }>(null);
  const [placeId, setPlaceId] = useState(String(ROGUE_LINEAGE));
  const [jobId, setJobId] = useState("");
  const [follow, setFollow] = useState("");
  const anchor = useRef<number | null>(null);
  const place = usePlaceInfo(placeId);

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
  const includeMain = !!snap?.settings.include_main_in_batch;
  const selectable = useMemo(() => visible.filter((a) => includeMain || !a.is_main).map((a) => a.id), [visible, includeMain]);
  const allOn = selectable.length > 0 && selectable.every((id) => selected.includes(id));
  const someOn = selected.length > 0 && !allOn;

  // Dev hook for screenshots: RAM_START_DIALOG=add|import opens a dialog.
  useEffect(() => {
    const on = (e: Event) => { const d = (e as CustomEvent<string>).detail; if (d === "add" || d === "import") setDialog({ kind: d }); };
    window.addEventListener("ram-start-dialog", on);
    return () => window.removeEventListener("ram-start-dialog", on);
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const el = e.target as HTMLElement;
      if (el.tagName === "INPUT" || el.tagName === "TEXTAREA") return;
      if (e.ctrlKey && e.key.toLowerCase() === "a") { e.preventDefault(); setSelected(selectable); }
      if (e.key === "Delete" && selected.length) setDialog({ kind: "remove", ids: selected });
      if (e.key === "Enter" && selected.length) void join();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectable, selected, placeId, jobId]);

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
  const toggle = (a: Account) => {
    setSelected(selected.includes(a.id) ? selected.filter((x) => x !== a.id) : [...selected, a.id]);
    anchor.current = a.id;
  };
  const toggleAll = () => setSelected(allOn ? [] : selectable);
  const toggleGroup = (list: Account[]) => {
    const ids = list.filter((a) => includeMain || !a.is_main).map((a) => a.id);
    const on = ids.length > 0 && ids.every((id) => selected.includes(id));
    setSelected(on ? selected.filter((id) => !ids.includes(id)) : Array.from(new Set([...selected, ...ids])));
  };

  const needCookieVault = (): boolean => {
    if (!vaultConfigured) { toast("Set up the cookie vault in Settings → Security first.", "warn"); return true; }
    if (!vaultUnlocked) { onNeedUnlock(); return true; }
    return false;
  };

  const join = async () => {
    if (!selected.length) { toast("Select an account to launch.", "warn"); return; }
    const pid = Number(placeId.trim().match(/\d{3,}/)?.[0] ?? 0);
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
    try { const note = await api.openBrowser(id); if (note) toast(note); } catch (e) { toast(String(e), "bad"); }
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

  const placeHint = (() => {
    switch (place.state) {
      case "loading": return <span className="muted">Looking up the place…</span>;
      case "ok": return <span><b>{place.game!.name}</b>{place.game!.creator ? <span className="muted"> · by {place.game!.creator}</span> : null}</span>;
      case "unknown": return <span className="bad">No Roblox place has that ID.</span>;
      case "error": return <span className="warn" title={place.error}>Could not look up the place right now.</span>;
      default: return <span className="muted">Type a place ID or paste a game link.</span>;
    }
  })();

  return (
    <div className="accounts">
      <div className="toolbar">
        <label className="check all" title={allOn ? "Clear selection" : "Select all shown accounts"}>
          <input type="checkbox" checked={allOn} ref={(el) => { if (el) el.indeterminate = someOn; }} onChange={toggleAll} />
          <span className="box" />
        </label>
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

      <div className="list">
        {accounts.length === 0 ? (
          <div className="empty"><div><b>No accounts yet.</b><br />Add one, or import from Roblox Account Manager.<br /><span className="muted">Then open the browser to sign in, or paste a cookie.</span></div></div>
        ) : visible.length === 0 ? (
          <div className="empty"><div>No accounts match.</div></div>
        ) : grouped.map(([g, list]) => {
          const gIds = list.filter((a) => includeMain || !a.is_main).map((a) => a.id);
          const gOn = gIds.length > 0 && gIds.every((id) => selected.includes(id));
          return (
            <div key={g || "__none"}>
              {(groups.length > 0) && (
                <div className="group-h">
                  <label className="check" title={gOn ? "Deselect this group" : "Select this group"}>
                    <input type="checkbox" checked={gOn} onChange={() => toggleGroup(list)} />
                    <span className="box" />
                  </label>
                  {g || "No group"} <span className="count">{list.length}</span>
                </div>
              )}
              {list.map((a) => {
                const ls = launchStatus[a.id];
                const sub = subtitle(a, ls);
                const on = selected.includes(a.id);
                return (
                  <div key={a.id} className={`acct ${on ? "selected" : ""}`} onClick={(e) => select(a, e)} onDoubleClick={() => { setSelected([a.id]); void join(); }}>
                    <label className="check" onClick={(e) => e.stopPropagation()} onDoubleClick={(e) => e.stopPropagation()}>
                      <input type="checkbox" checked={on} onChange={() => toggle(a)} />
                      <span className="box" />
                    </label>
                    <Avatar name={a.username} seed={a.id} src={a.avatar_url} />
                    <div className="grow" style={{ minWidth: 0 }}>
                      <div className="name ellipsis">{a.username}{a.is_main && <span className="main-tag">MAIN</span>}{a.favorite && <Star style={{ width: 12, height: 12, marginLeft: 6, color: "var(--warn)", verticalAlign: "-1px" }} />}{a.full_quality && !a.is_main && <Gauge style={{ width: 12, height: 12, marginLeft: 6, color: "var(--muted)", verticalAlign: "-1px" }} />}</div>
                      <div className={`sub ellipsis ${ls?.state === "Failed" ? "bad" : ""}`} title={sub}>{sub}</div>
                    </div>
                    <div><StatusBadge a={a} /></div>
                    <div className="right">{a.client === "Running" ? <span className="accent">● in game</span> : a.robux != null ? <span>R$ {a.robux.toLocaleString()}</span> : a.has_cookie ? <span title="Cookie stored"><KeyRound style={{ width: 13, height: 13 }} /></span> : ""}</div>
                    <div className="right">{ago(a.last_used)}</div>
                    <div className="right">{a.premium ? <Crown style={{ width: 14, height: 14, color: "var(--warn)" }} /> : null}</div>
                  </div>
                );
              })}
            </div>
          );
        })}
      </div>

      <aside className="details">
        {one ? (
          <Details a={one}
            onOpenBrowser={() => openBrowser(one.id)}
            onCapture={() => capture(one.id)}
            onCheck={() => check([one.id])}
            onSetCookie={() => { if (!needCookieVault()) setDialog({ kind: "cookie", id: one.id }); }}
            onReveal={() => { if (!needCookieVault()) setDialog({ kind: "reveal", id: one.id }); }}
            onRemoveCookie={() => setDialog({ kind: "removeCookie", id: one.id })}
            onRemove={() => setDialog({ kind: "remove", ids: [one.id] })} />
        ) : (
          <div className="empty"><div>{selected.length > 1 ? <><b>{selected.length} accounts selected</b><br /><span className="muted">Join Server launches them one by one{snap?.settings.launch_spacing_secs ? `, ${snap.settings.launch_spacing_secs}s apart` : ""}.</span><br /><br /><Btn sm onClick={() => check(selected)}><RefreshCw />Check selected</Btn></> : <>Select an account to see its details.<br /><span className="muted">Tick the boxes to pick several.</span></>}</div></div>
        )}
      </aside>

      <div className="launchbar">
        <div className="lgroup">
          <input className="input place mono" placeholder="Place ID" value={placeId} onChange={(e) => setPlaceId(e.target.value)} onKeyDown={(e) => e.key === "Enter" && join()} title="Place ID or game link" />
          <input className="input job mono" placeholder="Job ID (optional)" value={jobId} onChange={(e) => setJobId(e.target.value)} onKeyDown={(e) => e.key === "Enter" && join()} />
          <Btn kind="primary" onClick={join} disabled={!selected.length || place.state === "unknown"} title={selected.length ? `Launch ${selected.length} selected` : "Select accounts to launch"}><Play />Join Server</Btn>
        </div>
        <div className="lgroup">
          <input className="input user" placeholder="Username" value={follow} onChange={(e) => setFollow(e.target.value)} onKeyDown={(e) => e.key === "Enter" && doFollow()} />
          <Btn onClick={doFollow} disabled={!selected.length} title="Join the game this user is in"><UserPlus />Follow</Btn>
        </div>
        <div className="lhint">{placeHint}{selected.length > 0 && <span className="muted"> · {selected.length} selected</span>}</div>
      </div>

      {dialog?.kind === "add" && <AddAccountDialog onClose={() => setDialog(null)} onImport={() => setDialog({ kind: "import" })} />}
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
  const { refresh, toast, snap } = useApp();
  const [alias, setAlias] = useState(a.alias);
  const [notes, setNotes] = useState(a.notes);
  const [grp, setGrp] = useState(a.group);
  const [fps, setFps] = useState(a.fps_cap == null ? "" : String(a.fps_cap));
  useEffect(() => { setAlias(a.alias); setNotes(a.notes); setGrp(a.group); setFps(a.fps_cap == null ? "" : String(a.fps_cap)); }, [a.id, a.alias, a.notes, a.group, a.fps_cap]);
  const save = async (patch: Parameters<typeof api.updateAccount>[0]) => { try { await api.updateAccount(patch); await refresh(); } catch (e) { toast(String(e), "bad"); } };
  const t = tone(a.sign_in.state);
  const perfOn = !!snap?.graphics.enabled;
  const browserName = snap?.browser_name || "browser";
  return (
    <>
      <div className="head">
        <Avatar lg name={a.username} seed={a.id} src={a.avatar_url} />
        <div className="t grow">
          <b className="ellipsis">{a.username}</b>
          <span className="ellipsis">{a.display_name && a.display_name !== a.username ? `${a.display_name} · ` : ""}{a.roblox_user_id ? `ID ${a.roblox_user_id}` : "ID unknown"}</span>
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
        <Btn kind="primary" onClick={onOpenBrowser} title={`Open roblox.com in ${browserName}, signed in as this account (its own profile)`}><Globe />Open Browser</Btn>
        <Btn onClick={onCapture} title={`Save the session from the open ${browserName} window into the vault`}><ScanLine />Capture session</Btn>
        <Btn onClick={onCheck} disabled={a.checking}><RefreshCw />Check</Btn>
        <Btn onClick={onSetCookie}><KeyRound />Set cookie…</Btn>
        <Btn onClick={onReveal} disabled={!a.has_cookie}><Eye />Reveal cookie</Btn>
        <Btn kind="danger" onClick={onRemoveCookie} disabled={!a.has_cookie}><Trash2 />Remove cookie</Btn>
      </div>

      <Field label="Alias">
        <div className="inline"><input className="input" value={alias} onChange={(e) => setAlias(e.target.value)} onKeyDown={(e) => e.key === "Enter" && save({ id: a.id, alias })} placeholder="A name only you see" /><Btn sm onClick={() => save({ id: a.id, alias })} disabled={alias === a.alias}>Set</Btn></div>
      </Field>
      <Field label="Group">
        <div className="inline"><input className="input" value={grp} onChange={(e) => setGrp(e.target.value)} onKeyDown={(e) => e.key === "Enter" && save({ id: a.id, group: grp })} placeholder="e.g. Farm" list="ram-groups" /><Btn sm onClick={() => save({ id: a.id, group: grp })} disabled={grp === a.group}>Set</Btn></div>
      </Field>
      <Field label="Description">
        <textarea className="textarea" rows={3} value={notes} onChange={(e) => setNotes(e.target.value)} placeholder="Notes…" />
        <div><Btn sm onClick={() => save({ id: a.id, notes })} disabled={notes === a.notes}>Set description</Btn></div>
      </Field>
      <Toggle checked={a.is_main} onChange={async (v) => { await api.setMain(v ? a.id : null); await refresh(); }} label="Main account" hint="Sorted first, left out of Select All, and never given the performance settings." />
      <Toggle checked={a.favorite} onChange={(v) => save({ id: a.id, favorite: v })} label="Favorite" />
      {!a.is_main && (
        <>
          <Toggle checked={a.full_quality} onChange={(v) => save({ id: a.id, full_quality: v })} label="Full quality" hint={perfOn ? "Keeps the user's own graphics for this account; the FPS cap still applies." : "Performance settings are off (Settings → Performance), so this changes nothing yet."} />
          <Field label="FPS cap for this account" hint={perfOn ? "Blank uses the global cap." : "Takes effect when performance settings are on."}>
            <div className="inline"><input className="input" style={{ maxWidth: 110 }} type="number" min={5} max={360} placeholder="global" value={fps} onChange={(e) => setFps(e.target.value)} onKeyDown={(e) => e.key === "Enter" && save({ id: a.id, fps_cap: fps.trim() ? Math.max(5, Math.min(360, Number(fps))) : null, clear_fps_cap: !fps.trim() })} /><Btn sm onClick={() => save({ id: a.id, fps_cap: fps.trim() ? Math.max(5, Math.min(360, Number(fps))) : null, clear_fps_cap: !fps.trim() })} disabled={(fps.trim() ? Number(fps) : null) === a.fps_cap}>Set</Btn></div>
          </Field>
        </>
      )}
      <div><Btn kind="danger" sm onClick={onRemove}><Trash2 />Remove account</Btn></div>
    </>
  );
}
