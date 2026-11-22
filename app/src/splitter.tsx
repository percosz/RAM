// Draggable dividers: one between the sidebar and the content, one between
// the account list and the details pane. A divider only sets a CSS variable
// (the layout reads it) and remembers the width on this PC; double-click
// puts the default back.

const KINDS = {
  sidebar: { cssVar: "--sidebar-w", key: "ram.sidebar-w", min: 150, max: 340 },
  details: { cssVar: "--details-w", key: "ram.details-w", min: 260, max: 720 },
} as const;

/** The list keeps at least this much room when the details pane is widened. */
const LIST_MIN = 380;

/** Applies the widths saved earlier. Called once at start. */
export function restoreSplits() {
  for (const cfg of Object.values(KINDS)) {
    try {
      const w = Number(localStorage.getItem(cfg.key));
      if (w >= cfg.min && w <= cfg.max) document.documentElement.style.setProperty(cfg.cssVar, `${w}px`);
    } catch { /* storage unavailable: defaults */ }
  }
}

export function Splitter({ kind }: { kind: keyof typeof KINDS }) {
  const cfg = KINDS[kind];
  const onDown = (e: React.PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    e.preventDefault();
    const el = e.currentTarget;
    const box = el.parentElement!.getBoundingClientRect();
    let width = 0;
    const move = (ev: PointerEvent) => {
      const raw = kind === "sidebar" ? ev.clientX - box.left : box.right - ev.clientX;
      const max = kind === "sidebar" ? cfg.max : Math.max(cfg.min, Math.min(cfg.max, box.width - LIST_MIN));
      width = Math.round(Math.max(cfg.min, Math.min(max, raw)));
      document.documentElement.style.setProperty(cfg.cssVar, `${width}px`);
    };
    const up = () => {
      el.classList.remove("dragging");
      el.removeEventListener("pointermove", move);
      el.removeEventListener("pointerup", up);
      el.removeEventListener("pointercancel", up);
      if (width) { try { localStorage.setItem(cfg.key, String(width)); } catch { /* not remembered */ } }
    };
    el.setPointerCapture(e.pointerId);
    el.classList.add("dragging");
    el.addEventListener("pointermove", move);
    el.addEventListener("pointerup", up);
    el.addEventListener("pointercancel", up);
  };
  const reset = () => {
    document.documentElement.style.removeProperty(cfg.cssVar);
    try { localStorage.removeItem(cfg.key); } catch { /* nothing to forget */ }
  };
  return <div className={`splitter splitter-${kind}`} onPointerDown={onDown} onDoubleClick={reset} title="Drag to resize. Double-click for the default width." />;
}
