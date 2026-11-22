// The tutorial: a short guided tour that spotlights one part of the app at a
// time. It opens by itself on the first launch and again from the bottom of
// Settings. While it is open the app underneath cannot be clicked, so
// nothing is changed by accident.

import { useCallback, useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { Activity, Gauge, ListChecks, PanelRight, PartyPopper, Play, ShieldCheck, Sparkles, Stethoscope, UserPlus } from "lucide-react";
import { Btn } from "../components";

export type TourView = "accounts" | "clients" | "diagnostics" | "settings";

interface Step {
  /** Screen to show for this step. */
  view?: TourView;
  /** `data-tour` names to spotlight together; none centres the card. */
  target?: string[];
  /** Bring the target into view first (for things far down a page). */
  scroll?: boolean;
  /** Put the card under the target when there is room (for toolbar buttons). */
  below?: boolean;
  icon: ReactNode;
  title: string;
  body: ReactNode;
}

const STEPS: Step[] = [
  {
    icon: <Sparkles />,
    title: "Welcome to RAM",
    body: <>
      <p>RAM keeps your Roblox accounts in one place and launches several of them at the same time.</p>
      <p>This tour takes about a minute and points at where everything is. Skip it whenever you like: it is always at the bottom of Settings.</p>
    </>,
  },
  {
    view: "accounts", target: ["import", "add"], below: true, icon: <UserPlus />,
    title: "Add your accounts",
    body: <ul>
      <li><b>Add</b>: sign in with Roblox in your own browser, paste a cookie, or paste a whole list. <code>username:password</code> lines are signed in for you, one at a time.</li>
      <li><b>Import</b>: coming from Roblox Account Manager? RAM finds your <code>AccountData.json</code> or its backup, and unlocks it if it has a password.</li>
    </ul>,
  },
  {
    view: "accounts", target: ["list"], icon: <ListChecks />,
    title: "Your accounts",
    body: <>
      <p>Each row shows the avatar, username, alias and whether the sign-in is still valid.</p>
      <ul>
        <li><b>Tick boxes</b> pick several accounts.</li>
        <li><b>Drag</b> a row to reorder.</li>
        <li><b>Right-click</b> for the rest: copy the cookie or password, set as Main, move to a group, remove.</li>
        <li>The eye button hides names, avatars and Robux for screenshots.</li>
      </ul>
    </>,
  },
  {
    view: "accounts", target: ["launch"], icon: <Play />,
    title: "Join a game",
    body: <>
      <p>Type a <b>Place ID</b> (RAM shows the game's name), tick the accounts and press <b>Join Server</b>. They start one after another.</p>
      <p><b>Follow</b> joins the server a user is playing in.</p>
    </>,
  },
  {
    view: "accounts", target: ["details"], icon: <PanelRight />,
    title: "One account in detail",
    body: <>
      <p>Click an account to open it here: alias and description, its own FPS cap, <b>Check</b> the sign-in, and <b>Open Browser</b> signed in as that account in your own browser.</p>
      <p>Mark your own account as <b>Main</b>. Main is left out of Select All and is never slowed down.</p>
    </>,
  },
  {
    view: "clients", target: ["nav-clients"], icon: <Activity />,
    title: "Running clients",
    body: <p>Every Roblox client that is open, with its account and memory use. Close one, or use <b>Minimize all</b> and <b>Restore all</b>.</p>,
  },
  {
    view: "diagnostics", target: ["nav-diagnostics"], icon: <Stethoscope />,
    title: "When a client drops",
    body: <p><b>Crash history</b> records why a client ended: disconnected by Roblox, crashed, or replaced by another sign-in, with the reason and how long it ran. The <b>debug log</b> is here too.</p>,
  },
  {
    view: "settings", target: ["perf"], scroll: true, icon: <Gauge />,
    title: "Make alts lighter",
    body: <>
      <p>Pick a preset to cap the FPS and lower the graphics of alt clients. Your own Roblox settings are put back right after each launch.</p>
      <p>Further down, <b>Running clients</b> can minimize, shrink or mute alts when they start and limit their CPU in the background. Main is always left alone.</p>
    </>,
  },
  {
    view: "accounts", target: ["vault"], icon: <ShieldCheck />,
    title: "Cookies stay encrypted",
    body: <p>Sign-in cookies and saved passwords live in an encrypted vault on this PC. Copying one asks for Windows Hello or your master password. This button shows the vault's state; click it to set it up or unlock it.</p>,
  },
  {
    icon: <PartyPopper />,
    title: "You're set",
    body: <>
      <p>Add an account to get started.</p>
      <p>New versions are announced when RAM starts, and this tutorial is at the bottom of <b>Settings</b> whenever you want it again.</p>
    </>,
  },
];

export const TOUR_STEPS = STEPS.length;

interface Box { left: number; top: number; width: number; height: number; }

const PAD = 6;      // room between the target and the spotlight ring
const GAP = 14;     // room between the ring and the card
const EDGE = 12;    // closest the card comes to the window edge

/** The rectangle around every visible target of a step, kept inside the window. */
function boxOf(names: string[] | undefined): Box | null {
  let l = Infinity, t = Infinity, r = -Infinity, b = -Infinity;
  for (const name of names ?? []) {
    const el = document.querySelector(`[data-tour="${name}"]`);
    if (!el) continue;
    const q = el.getBoundingClientRect();
    if (q.width === 0 && q.height === 0) continue;
    l = Math.min(l, q.left); t = Math.min(t, q.top); r = Math.max(r, q.right); b = Math.max(b, q.bottom);
  }
  if (!Number.isFinite(l)) return null;
  l = Math.max(4, l - PAD); t = Math.max(4, t - PAD);
  r = Math.min(window.innerWidth - 4, r + PAD); b = Math.min(window.innerHeight - 4, b + PAD);
  if (r - l < 8 || b - t < 8) return null;
  return { left: Math.round(l), top: Math.round(t), width: Math.round(r - l), height: Math.round(b - t) };
}

const clamp = (v: number, lo: number, hi: number) => Math.max(lo, Math.min(hi, v));

/** Where the card goes: beside the target where there is room, else in a corner. */
function placeCard(box: Box | null, cw: number, ch: number, preferBelow = false): { left: number; top: number } {
  const vw = window.innerWidth, vh = window.innerHeight;
  if (!box) return { left: Math.round((vw - cw) / 2), top: Math.round((vh - ch) / 2) };
  const midX = clamp(box.left + box.width / 2 - cw / 2, EDGE, vw - cw - EDGE);
  const alongY = clamp(box.top, EDGE, vh - ch - EDGE);
  const right = box.left + box.width + GAP, left = box.left - GAP - cw;
  const below = box.top + box.height + GAP, above = box.top - GAP - ch;
  if (preferBelow && below + ch <= vh - EDGE) return { left: clamp(box.left + box.width - cw, EDGE, vw - cw - EDGE), top: below };
  if (right + cw <= vw - EDGE) return { left: right, top: alongY };
  if (left >= EDGE) return { left, top: alongY };
  if (below + ch <= vh - EDGE) return { left: midX, top: below };
  if (above >= EDGE) return { left: midX, top: above };
  // The target fills the window: sit in its bottom-right corner.
  return { left: vw - cw - 24, top: vh - ch - 24 };
}

export function Tour({ start = 0, setView, onDone }: { start?: number; setView: (v: TourView) => void; onDone: () => void }) {
  const [i, setI] = useState(() => clamp(start, 0, STEPS.length - 1));
  const [box, setBox] = useState<Box | null>(null);
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);
  const [ready, setReady] = useState(false);
  const card = useRef<HTMLDivElement>(null);
  const step = STEPS[i]!;
  const last = i === STEPS.length - 1;

  const measure = useCallback(() => {
    const next = boxOf(STEPS[i]!.target);
    setBox((cur) => (cur && next && cur.left === next.left && cur.top === next.top && cur.width === next.width && cur.height === next.height) || (!cur && !next) ? cur : next);
  }, [i]);

  // A new step: show its screen, wait for it to be laid out, then find the
  // target. The card stays hidden for those two frames so it never flashes
  // in the wrong place.
  useLayoutEffect(() => {
    setReady(false);
    if (step.view) setView(step.view);
    (document.activeElement as HTMLElement | null)?.blur?.();
    let a = 0, b = 0;
    a = requestAnimationFrame(() => {
      b = requestAnimationFrame(() => {
        if (step.scroll && step.target?.[0]) document.querySelector(`[data-tour="${step.target[0]}"]`)?.scrollIntoView({ block: "center", inline: "nearest" });
        measure();
        setReady(true);
      });
    });
    return () => { cancelAnimationFrame(a); cancelAnimationFrame(b); };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [i]);

  // Follow the layout while the step is open (window resized, data loaded).
  useEffect(() => {
    const t = window.setInterval(measure, 250);
    window.addEventListener("resize", measure);
    return () => { window.clearInterval(t); window.removeEventListener("resize", measure); };
  }, [measure]);

  useLayoutEffect(() => {
    if (!ready || !card.current) return;
    const next = placeCard(box, card.current.offsetWidth, card.current.offsetHeight, STEPS[i]!.below);
    setPos((cur) => (cur && cur.left === next.left && cur.top === next.top ? cur : next));
  }, [box, ready, i]);

  const next = useCallback(() => { if (last) onDone(); else setI((n) => n + 1); }, [last, onDone]);
  const back = useCallback(() => setI((n) => Math.max(0, n - 1)), []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onDone();
      else if (e.key === "ArrowRight" || e.key === "Enter") next();
      else if (e.key === "ArrowLeft") back();
      else return;
      e.preventDefault();
      e.stopPropagation();
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [next, back, onDone]);

  const shown = ready && pos !== null;
  return (
    <div className={`tour ${box ? "" : "dim"}`} role="dialog" aria-modal="true" aria-label={`Tutorial: ${step.title}`}>
      {box && <div className="tour-hole" style={{ left: box.left, top: box.top, width: box.width, height: box.height }} />}
      <div ref={card} key={i} className="tour-card" style={{ left: pos?.left ?? 0, top: pos?.top ?? 0, visibility: shown ? "visible" : "hidden" }}>
        <div className="tour-head">
          <span className="tour-icon">{step.icon}</span>
          <div className="grow">
            <div className="tour-count">Step {i + 1} of {STEPS.length}</div>
            <div className="tour-title">{step.title}</div>
          </div>
        </div>
        <div className="tour-body">{step.body}</div>
        <div className="tour-dots" aria-hidden>
          {STEPS.map((_, n) => <i key={n} className={n === i ? "on" : n < i ? "done" : ""} />)}
        </div>
        <div className="tour-foot">
          {!last && <Btn sm kind="ghost" onClick={onDone}>Skip tutorial</Btn>}
          <span className="grow" />
          {i > 0 && <Btn sm onClick={back}>Back</Btn>}
          <Btn sm kind="primary" onClick={next}>{last ? "Done" : i === 0 ? "Start the tour" : "Next"}</Btn>
        </div>
      </div>
    </div>
  );
}
