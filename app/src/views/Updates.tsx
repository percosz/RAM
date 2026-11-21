// The update offer (a small dialog from the sidebar pill) and the one-time
// "What's new" after an update. Nothing here blocks the app: the pill waits
// until clicked, and What's new is dismissed with one click.

import { useEffect, useState, type ReactNode } from "react";
import { api } from "../api";
import { Btn, Modal } from "../components";
import { useApp } from "../store";

/** Renders release notes written in simple Markdown (headings, bullets, bold, code). */
export function Notes({ text }: { text: string }) {
  const out: ReactNode[] = [];
  let list: ReactNode[] = [];
  const flush = () => { if (list.length) { out.push(<ul key={`ul${out.length}`}>{list}</ul>); list = []; } };
  const inline = (s: string): ReactNode[] => s.split(/(`[^`]+`|\*\*[^*]+\*\*)/g).filter(Boolean).map((p, i) =>
    p.startsWith("`") ? <code key={i} className="mono">{p.slice(1, -1)}</code> : p.startsWith("**") ? <b key={i}>{p.slice(2, -2)}</b> : <span key={i}>{p}</span>);
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.trim();
    if (!line) continue;
    const h = /^#{1,4}\s+(.*)$/.exec(line);
    if (h) { flush(); out.push(<h3 key={`h${out.length}`}>{h[1]}</h3>); continue; }
    const li = /^[-*]\s+(.*)$/.exec(line);
    if (li) { list.push(<li key={`li${out.length}-${list.length}`}>{inline(li[1]!)}</li>); continue; }
    flush();
    out.push(<p key={`p${out.length}`} style={{ margin: "4px 0", fontSize: 13 }}>{inline(line)}</p>);
  }
  flush();
  return <div className="notes">{out.length ? out : <p className="muted" style={{ margin: 0 }}>No notes for this release.</p>}</div>;
}

export function UpdateDialog({ onClose }: { onClose: () => void }) {
  const { update, installUpdate, dismissUpdate, snap } = useApp();
  if (!update) return null;
  const busy = update.state === "downloading" || update.state === "installing";
  const running = snap?.accounts.filter((a) => a.client === "Running").length ?? 0;
  const RELEASES = `https://github.com/${snap?.github_repo ?? ""}/releases`;
  return (
    <Modal title={`RAM ${update.version} is available`} onClose={busy ? () => {} : onClose} wide
      footer={<>
        {!busy && <Btn onClick={() => { dismissUpdate(); onClose(); }}>Later</Btn>}
        {update.portable
          ? <Btn kind="primary" onClick={() => api.openUrl(RELEASES).catch(() => {})}>Open the release page</Btn>
          : <Btn kind="primary" onClick={installUpdate} disabled={busy}>{update.state === "downloading" ? `Downloading… ${Math.round(update.progress * 100)}%` : update.state === "installing" ? "Installing…" : "Update now"}</Btn>}
      </>}>
      <p className="muted" style={{ margin: 0 }}>
        {update.portable
          ? "This copy of RAM runs from a folder (no installer), so it cannot update itself. Download the new version from the release page and replace RAM.exe."
          : `RAM downloads the signed installer from GitHub, closes, installs, and starts again by itself.${running ? ` ${running} Roblox client${running === 1 ? "" : "s"} keep running.` : ""}`}
      </p>
      {update.state === "failed" && <div className="bad">Update failed: {update.error}</div>}
      <div style={{ maxHeight: 320, overflow: "auto", padding: "8px 12px", border: "1px solid var(--border)", borderRadius: "var(--radius-sm)", background: "var(--surface-2)" }}>
        <Notes text={update.notes} />
      </div>
      {update.date && <div className="hint">Published {update.date.slice(0, 10)}</div>}
    </Modal>
  );
}

/** Shown once after RAM has been updated: the notes of the version now running. */
export function WhatsNewDialog() {
  const { snap, refresh } = useApp();
  const [notes, setNotes] = useState<string | null>(null);
  const [open, setOpen] = useState(true);
  useEffect(() => { api.releaseNotes().then(setNotes).catch(() => setNotes("")); }, []);
  if (!open || !snap || notes === null) return null;
  const close = async () => { setOpen(false); await api.markSeenVersion().catch(() => {}); void refresh(); };
  return (
    <Modal title={`What's new in RAM ${snap.version}`} onClose={close} footer={<Btn kind="primary" onClick={close}>Got it</Btn>}>
      <Notes text={notes} />
    </Modal>
  );
}
