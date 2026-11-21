// A small context menu with one level of submenus, positioned at the pointer
// and kept inside the window.

import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { ChevronRight } from "lucide-react";

export interface MenuItem {
  label?: string;
  icon?: ReactNode;
  onClick?: () => void;
  danger?: boolean;
  disabled?: boolean;
  hint?: string;
  sub?: MenuItem[];
  /** A divider line instead of an item. */
  sep?: boolean;
}

function Items({ items, onClose }: { items: MenuItem[]; onClose: () => void }) {
  const [open, setOpen] = useState<number | null>(null);
  return (
    <>
      {items.map((it, i) => it.sep ? <div key={i} className="menu-sep" /> : (
        <div key={i} className={`menu-item ${it.danger ? "danger" : ""} ${it.disabled ? "disabled" : ""}`}
          onMouseEnter={() => setOpen(it.sub ? i : null)}
          onClick={(e) => { e.stopPropagation(); if (it.disabled || it.sub) return; it.onClick?.(); onClose(); }}>
          <span className="mi-icon">{it.icon}</span>
          <span className="grow ellipsis">{it.label}</span>
          {it.hint && <span className="mi-hint">{it.hint}</span>}
          {it.sub && <ChevronRight className="mi-arrow" />}
          {it.sub && open === i && <Sub items={it.sub} onClose={onClose} />}
        </div>
      ))}
    </>
  );
}

function Sub({ items, onClose }: { items: MenuItem[]; onClose: () => void }) {
  const ref = useRef<HTMLDivElement>(null);
  const [flip, setFlip] = useState({ x: false, y: 0 });
  useLayoutEffect(() => {
    const r = ref.current?.getBoundingClientRect();
    if (!r) return;
    setFlip({ x: r.right > window.innerWidth - 4, y: Math.min(0, window.innerHeight - 4 - r.bottom) });
  }, []);
  return (
    <div ref={ref} className="menu sub" style={{ [flip.x ? "right" : "left"]: "calc(100% - 4px)", top: -5 + flip.y }} onClick={(e) => e.stopPropagation()}>
      <Items items={items} onClose={onClose} />
    </div>
  );
}

export function ContextMenu({ x, y, items, onClose }: { x: number; y: number; items: MenuItem[]; onClose: () => void }) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ left: x, top: y });
  useLayoutEffect(() => {
    const r = ref.current?.getBoundingClientRect();
    if (!r) return;
    setPos({ left: Math.max(4, Math.min(x, window.innerWidth - r.width - 4)), top: Math.max(4, Math.min(y, window.innerHeight - r.height - 4)) });
  }, [x, y]);
  useEffect(() => {
    const close = () => onClose();
    const key = (e: KeyboardEvent) => { if (e.key === "Escape") onClose(); };
    window.addEventListener("mousedown", close);
    window.addEventListener("blur", close);
    window.addEventListener("resize", close);
    window.addEventListener("keydown", key);
    return () => { window.removeEventListener("mousedown", close); window.removeEventListener("blur", close); window.removeEventListener("resize", close); window.removeEventListener("keydown", key); };
  }, [onClose]);
  return (
    <div ref={ref} className="menu" style={pos} onMouseDown={(e) => e.stopPropagation()} onContextMenu={(e) => e.preventDefault()}>
      <Items items={items} onClose={onClose} />
    </div>
  );
}
