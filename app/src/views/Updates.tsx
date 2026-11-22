// The update offer and the one-time "What's new" after an update. The offer
// opens by itself when a new version is found at launch (Update now or
// Later) and from the sidebar pill, which stays after "Later" so the update
// is one click away for the rest of the session.

import { useEffect, useState, type ReactNode } from "react";
import { api } from "../api";
import { Btn, Modal } from "../components";
import { useApp } from "../store";

/** Renders release notes written in simple Markdown (headings, bullets, bold, code). */
export function Notes({ text }: { text: string }) {
  const out: ReactNode[] = [];
  let list: ReactNode[] = [];
  const flush = () => { if (list.length) { out.push(<ul key={`ul${out.length}`}>{list}</ul>); list = []; } };
  const inline = (s: string): ReactNode[] => s.split(/(`[^`]+`|\*\*[^*]+\*\*|\*[^*\s][^*]*\*)/g).filter(Boolean).map((p, i) =>
    p.startsWith("`") ? <code key={i} className="mono">{p.slice(1, -1)}</code> : p.startsWith("**") ? <b key={i}>{p.slice(2, -2)}</b> : p.startsWith("*") && p.length > 2 ? <i key={i}>{p.slice(1, -1)}</i> : <span key={i}>{p}</span>);
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
  const { update, installUpdate, snap } = useApp();
  if (!update) return null;
  const busy = update.state === "downloading" || update.state === "installing";
  const running = snap?.accounts.filter((a) => a.client === "Running").length ?? 0;
  const RELEASES = `https://github.com/${snap?.github_repo ?? ""}/releases`;
  return (
    <Modal title={`Update available: RAM ${update.version}`} onClose={busy ? () => {} : onClose} wide
      footer={<>
        {!busy && <Btn onClick={onClose}>Later</Btn>}
        {update.portable
          ? <Btn kind="primary" onClick={() => api.openUrl(RELEASES).catch(() => {})}>Open the release page</Btn>
          : <Btn kind="primary" onClick={installUpdate} disabled={busy}>{update.state === "downloading" ? `Downloading… ${Math.round(update.progress * 100)}%` : update.state === "installing" ? "Installing…" : "Update now"}</Btn>}
      </>}>
      <p className="muted" style={{ margin: 0 }}>
        You have RAM {snap?.version}.{" "}
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

/** Shown once at the first start after an update: the notes of every version since the one last seen. */
export function WhatsNewDialog() {
  const { snap, refresh } = useApp();
  const [notes, setNotes] = useState<string | null>(null);
  const [open, setOpen] = useState(true);
  useEffect(() => { api.releaseNotes().then(setNotes).catch(() => setNotes("")); }, []);
  const close = async () => { setOpen(false); await api.markSeenVersion().catch(() => {}); void refresh(); };
  // Nothing written for this version: record it as seen without a dialog.
  useEffect(() => { if (notes === "") void close(); }, [notes]); // eslint-disable-line react-hooks/exhaustive-deps
  if (!open || !snap || !notes) return null;
  return (
    <Modal title={`What's new in RAM ${snap.version}`} onClose={close} wide={notes.includes("\n## ")} footer={<Btn kind="primary" onClick={close}>Got it</Btn>}>
      {/* The notes scroll; the title and "Got it" stay in view. */}
      <div style={{ maxHeight: "58vh", overflow: "auto", paddingRight: 6 }}><Notes text={notes} /></div>
    </Modal>
  );
}
