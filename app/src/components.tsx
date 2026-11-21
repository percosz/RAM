// Small shared components: buttons, fields, toggles, badges, avatars, modals, toasts.

import { useEffect, type ReactNode } from "react";
import { X } from "lucide-react";
import type { Account, SignInState } from "./api";
import { signInLabel } from "./api";

export function Btn(props: React.ButtonHTMLAttributes<HTMLButtonElement> & { kind?: "primary" | "ghost" | "danger"; sm?: boolean; icon?: boolean }) {
  const { kind, sm, icon, className, ...rest } = props;
  const cls = ["btn", kind ?? "", sm ? "sm" : "", icon ? "icon" : "", className ?? ""].filter(Boolean).join(" ");
  return <button type="button" className={cls} {...rest} />;
}

export function Field({ label, hint, children }: { label: string; hint?: string; children: ReactNode }) {
  return (
    <div className="field">
      <label>{label}</label>
      {children}
      {hint && <div className="hint">{hint}</div>}
    </div>
  );
}

export function Toggle({ checked, onChange, label, hint }: { checked: boolean; onChange: (v: boolean) => void; label: string; hint?: string }) {
  return (
    <div>
      <label className="toggle">
        <input type="checkbox" checked={checked} onChange={(e) => onChange(e.target.checked)} />
        <span className="track" />
        <span>{label}</span>
      </label>
      {hint && <div className="hint" style={{ marginLeft: 46 }}>{hint}</div>}
    </div>
  );
}

export function tone(state: SignInState): "good" | "bad" | "warn" | "muted" {
  switch (state) {
    case "Verified": return "good";
    case "SignedOut": return "bad";
    case "VerificationRequired": case "CaptchaRequired": return "warn";
    default: return "muted";
  }
}

export function StatusBadge({ a }: { a: Account }) {
  if (a.checking) return <span className="badge muted"><span className="dot" />Checking…</span>;
  const t = tone(a.sign_in.state);
  return <span className={`badge ${t}`} title={a.sign_in.detail}><span className="dot" />{signInLabel(a.sign_in.state)}</span>;
}

const COLORS = ["#3e73b5", "#6ca84f", "#b56f3e", "#4f67a8", "#3e73b5", "#a87b4f", "#a84f92", "#6e4fa8", "#9ca84f", "#4fa86e"];

export function initials(name: string): string {
  const words = name.split(/[^a-zA-Z0-9]+/).filter(Boolean).slice(0, 2);
  const s = words.map((w) => w[0]!.toUpperCase()).join("");
  return s || "?";
}

export function Avatar({ name, seed, lg, src }: { name: string; seed: number; lg?: boolean; src?: string | null }) {
  const bg = COLORS[seed % COLORS.length];
  return (
    <span className={`avatar ${lg ? "lg" : ""}`} style={{ background: bg }} aria-hidden>
      {src ? <img src={src} alt="" /> : initials(name)}
    </span>
  );
}

export function Modal({ title, onClose, children, footer, wide }: { title: string; onClose: () => void; children: ReactNode; footer?: ReactNode; wide?: boolean }) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") onClose(); };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);
  return (
    <div className="backdrop" onMouseDown={(e) => { if (e.target === e.currentTarget) onClose(); }}>
      <div className={`modal ${wide ? "wide" : ""}`} role="dialog" aria-modal="true" aria-label={title}>
        <div className="modal-h row">
          <span className="grow">{title}</span>
          <Btn kind="ghost" icon sm onClick={onClose} aria-label="Close"><X /></Btn>
        </div>
        <div className="modal-b">{children}</div>
        {footer && <div className="modal-f">{footer}</div>}
      </div>
    </div>
  );
}

export interface ToastItem { id: number; text: string; tone?: "good" | "bad" | "warn"; }

export function Toasts({ items, dismiss }: { items: ToastItem[]; dismiss: (id: number) => void }) {
  return (
    <div className="toasts">
      {items.map((t) => (
        <div key={t.id} className={`toast ${t.tone ?? ""}`} onClick={() => dismiss(t.id)}>
          <span className="grow">{t.text}</span>
        </div>
      ))}
    </div>
  );
}
