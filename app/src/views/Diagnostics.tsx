// Diagnostics: how Roblox clients ended (crash history) and RAM's own log.

import { useEffect, useMemo, useState } from "react";
import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { Download, FolderOpen, RefreshCw, Trash2, Copy, FileText } from "lucide-react";
import { api, endKindLabel, type CrashRecord, type EndKind, type LogLine } from "../api";
import { Avatar, Btn } from "../components";
import { useApp } from "../store";

function when(unix: number): string {
  const d = new Date(unix * 1000);
  const diff = Math.max(0, Math.floor(Date.now() / 1000) - unix);
  if (diff < 3600) return `${Math.max(1, Math.floor(diff / 60))} min ago`;
  if (diff < 86400) return `${Math.floor(diff / 3600)} h ago`;
  if (diff < 7 * 86400) return `${Math.floor(diff / 86400)} d ago`;
  return d.toLocaleDateString();
}
function dur(secs: number): string {
  const m = Math.floor(secs / 60);
  return m < 1 ? `${secs}s` : m < 60 ? `${m}m` : `${Math.floor(m / 60)}h ${m % 60}m`;
}
function toneOf(k: EndKind): string {
  return k === "Crashed" || k === "SignedOut" ? "bad" : k === "Disconnected" || k === "Replaced" ? "warn" : "muted";
}

export function DiagnosticsView() {
  const { toast, clearCrashCount, accounts, nameOf } = useApp();
  const [tab, setTab] = useState<"crashes" | "log">("crashes");
  useEffect(() => { clearCrashCount(); }, [clearCrashCount]);
  return (
    <div className="diag">
      <div className="tabs">
        <button className={tab === "crashes" ? "active" : ""} onClick={() => setTab("crashes")}>Crash history</button>
        <button className={tab === "log" ? "active" : ""} onClick={() => setTab("log")}>Debug log</button>
      </div>
      {tab === "crashes" ? <Crashes toast={toast} accountNames={new Map(accounts.map((a) => [a.id, nameOf(a.username)]))} nameOf={nameOf} /> : <DebugLog toast={toast} />}
    </div>
  );
}

function Crashes({ toast, accountNames, nameOf }: { toast: (t: string, tone?: "good" | "bad" | "warn") => void; accountNames: Map<number, string>; nameOf: (u: string) => string }) {
  const [rows, setRows] = useState<CrashRecord[]>([]);
  const [kind, setKind] = useState<"" | EndKind>("");
  const [acct, setAcct] = useState<number | "">("");
  const load = async () => { try { setRows(await api.crashes()); } catch (e) { toast(String(e), "bad"); } };
  useEffect(() => {
    void load();
    let un: (() => void) | undefined;
    api.onCrashesChanged(() => { void load(); }).then((u) => { un = u; });
    return () => { un?.(); };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  const shown = rows.filter((r) => (!kind || r.kind === kind) && (acct === "" || r.account === acct));
  const kinds = useMemo(() => Array.from(new Set(rows.map((r) => r.kind))), [rows]);
  const accts = useMemo(() => Array.from(new Set(rows.map((r) => r.account).filter((x): x is number => x != null))), [rows]);
  const exportCsv = async () => {
    const path = await saveDialog({ defaultPath: `RAM-crashes-${new Date().toISOString().slice(0, 10)}.csv`, filters: [{ name: "CSV", extensions: ["csv"] }] });
    if (!path) return;
    try { await api.crashesExport(path); toast("Exported.", "good"); } catch (e) { toast(String(e), "bad"); }
  };
  return (
    <>
      <div className="bar">
        <span className="muted" style={{ fontSize: 12 }}>How your clients ended: disconnects, crashes and closes RAM saw. The newest 200 are kept on this PC. A client RAM did not launch is matched by its log's start time.</span>
        <span className="grow" />
        <select className="select" value={kind} onChange={(e) => setKind(e.target.value as "" | EndKind)}>
          <option value="">All kinds</option>
          {kinds.map((k) => <option key={k} value={k}>{endKindLabel(k)}</option>)}
        </select>
        <select className="select" value={acct} onChange={(e) => setAcct(e.target.value === "" ? "" : Number(e.target.value))}>
          <option value="">All accounts</option>
          {accts.map((id) => <option key={id} value={id}>{accountNames.get(id) ?? nameOf(rows.find((r) => r.account === id)?.username ?? "") ?? `#${id}`}</option>)}
        </select>
        <Btn sm onClick={load}><RefreshCw />Refresh</Btn>
        <Btn sm onClick={exportCsv} disabled={!rows.length}><Download />Export CSV</Btn>
        <Btn sm kind="danger" onClick={async () => { await api.crashesClear(); void load(); }} disabled={!rows.length}><Trash2 />Clear</Btn>
      </div>
      <div className="body">
        {shown.length === 0 ? (
          <div className="empty"><div><b>Nothing recorded yet.</b><br /><span className="muted">When a Roblox client disconnects, crashes or is closed, it shows up here with the reason from its log.</span></div></div>
        ) : shown.map((r) => (
          <div key={r.id} className="crash">
            <div className="who"><Avatar name={r.username || "?"} seed={r.account ?? 0} />{r.username ? nameOf(r.username) : <span className="muted">Unknown account</span>}</div>
            <div className={`kind ${toneOf(r.kind)}`}>{endKindLabel(r.kind)}</div>
            <div className="when">{when(r.at)}</div>
            <div className="meta">{r.place_name || (r.place_id ? `Place ${r.place_id}` : "No game joined")} · after {dur(r.uptime_secs)} · PID {r.pid}</div>
            {r.reason && <div className="reason">{r.reason}</div>}
            <div className="act">{r.has_log && <Btn sm onClick={() => api.crashOpenLog(r.id).catch((e) => toast(String(e), "bad"))} title="Opens the raw Roblox log file. It can contain session details; do not share it as is."><FileText />Open log</Btn>}</div>
          </div>
        ))}
      </div>
    </>
  );
}

function DebugLog({ toast }: { toast: (t: string, tone?: "good" | "bad" | "warn") => void }) {
  const [lines, setLines] = useState<LogLine[]>([]);
  const [on, setOn] = useState<Record<LogLine["level"], boolean>>({ error: true, warn: true, info: true, debug: false });
  const [filter, setFilter] = useState("");
  const load = async () => { try { setLines(await api.logTail(3000)); } catch (e) { toast(String(e), "bad"); } };
  useEffect(() => { void load(); const t = window.setInterval(load, 4000); return () => window.clearInterval(t); /* eslint-disable-line react-hooks/exhaustive-deps */ }, []);
  const counts = useMemo(() => lines.reduce((m, l) => { m[l.level] = (m[l.level] ?? 0) + 1; return m; }, {} as Record<string, number>), [lines]);
  const f = filter.trim().toLowerCase();
  const shown = lines.filter((l) => on[l.level] && (!f || l.text.toLowerCase().includes(f)));
  const copyDiag = async () => {
    try { await writeText(await api.diagnosticsText()); toast("Diagnostics copied (secrets and names redacted).", "good"); } catch (e) { toast(String(e), "bad"); }
  };
  return (
    <>
      <div className="bar">
        {(["error", "warn", "info", "debug"] as const).map((lv) => (
          <button key={lv} className={`chip ${on[lv] ? "on" : ""}`} onClick={() => setOn({ ...on, [lv]: !on[lv] })}>
            <span className={lv === "error" ? "bad" : lv === "warn" ? "warn" : lv === "info" ? "accent" : "muted"}>●</span> {lv[0]!.toUpperCase() + lv.slice(1)} <b>{counts[lv] ?? 0}</b>
          </button>
        ))}
        <input className="input" placeholder="Filter lines…" value={filter} onChange={(e) => setFilter(e.target.value)} />
        <span className="grow" />
        <Btn sm onClick={copyDiag} title="RAM version, settings, launcher and the latest log lines, with cookies, names and paths masked"><Copy />Copy diagnostics</Btn>
        <Btn sm onClick={() => api.openLogFolder().catch((e) => toast(String(e), "bad"))}><FolderOpen />Open log folder</Btn>
      </div>
      <div className="body">
        <div className="log">
          {shown.length === 0 ? <span className="muted">Nothing to show.</span> : shown.map((l, i) => <span key={i} className={`ln ${l.level}`}>{l.text}</span>)}
        </div>
      </div>
    </>
  );
}
