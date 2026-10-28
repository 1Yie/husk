"use client";

import {
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type PointerEvent as ReactPointerEvent,
} from "react";
import s from "./reasoning-effort.module.css";

const DEFAULT_STOPS = ["Low", "Medium", "High", "Extra High"] as const;
const DEFAULT_LABEL = "4.7";
const TRACK = 224;
const HEIGHT = 32;
const RADIUS = HEIGHT / 2;
const BUMP_H = 20;
const REST_W = 27.2;
const DRAG_W = 33.6;
const REST_H = 27.2;
const EDGE_PAD = (HEIGHT - REST_H) / 2;
const FAST_FRONT_SPARKLES = [
  { x: 4, y: 6, size: 3.5, delay: 20 },
  { x: 15, y: 20, size: 2.5, delay: 130 },
  { x: 1, y: 15, size: 4, delay: 210 },
  { x: 25, y: 9, size: 2, delay: 80 },
  { x: 10, y: 27, size: 3, delay: 300 },
  { x: 32, y: 23, size: 2, delay: 230 },
  { x: 18, y: 3, size: 2.5, delay: 330 },
  { x: 38, y: 15, size: 3, delay: 180 },
] as const;
const FAST_POINTS = [
  { x: 9, y: 32, size: 1.3, duration: 3.7, delay: -1.1 },
  { x: 19, y: 74, size: 1.7, duration: 4.8, delay: -3.2 },
  { x: 29, y: 21, size: 1.2, duration: 3.1, delay: -2.1 },
  { x: 37, y: 58, size: 1.8, duration: 5.2, delay: -0.8 },
  { x: 47, y: 83, size: 1.1, duration: 4.1, delay: -2.7 },
  { x: 54, y: 35, size: 1.5, duration: 3.4, delay: -1.6 },
  { x: 65, y: 67, size: 1.2, duration: 4.6, delay: -3.6 },
  { x: 73, y: 18, size: 1.7, duration: 5.4, delay: -2.2 },
  { x: 82, y: 49, size: 1.1, duration: 3.9, delay: -0.4 },
  { x: 92, y: 78, size: 1.4, duration: 4.3, delay: -3.1 },
] as const;

function clamp(n: number, min: number, max: number) {
  return Math.min(max, Math.max(min, n));
}

function centerFor(value: number, count: number, track: number) {
  const span = track - RADIUS * 2;
  return RADIUS + (value / Math.max(1, count - 1)) * span;
}

function easeOut(t: number) {
  return 1 - (1 - t) ** 3;
}

const LEFT_SHOULDER = 23.572;
/** Space from each glyph edge to the foot of the label shoulder. */
const TEXT_SIDE = 20;
/** How far the outer foot walks down the cap once it passes the crown. */
const PHI_MAX = 0.6;
const PHI_REACH = 36;

function plateauFor(textW: number) {
  return Math.max(8, textW + TEXT_SIDE * 2 - LEFT_SHOULDER * 2);
}

function smoothstep(t: number) {
  const x = clamp(t, 0, 1);
  return x * x * (3 - 2 * x);
}

function capFoot(side: -1 | 1, phi: number, track: number) {
  const crown = side === -1 ? RADIUS : track - RADIUS;
  return {
    x: crown + side * RADIUS * Math.sin(phi),
    y: BUMP_H + RADIUS * (1 - Math.cos(phi)),
  };
}

function shellGeom(cx: number, plateauW: number, track: number) {
  const crownL = RADIUS;
  const crownR = track - RADIUS;
  let topL = cx - plateauW / 2;
  let topR = topL + plateauW;
  let phiL = 0;
  let phiR = 0;

  const overflowL = crownL - (topL - LEFT_SHOULDER);
  if (overflowL > 0) {
    phiL = PHI_MAX * smoothstep(overflowL / PHI_REACH);
    const foot = capFoot(-1, phiL, track);
    const minTopL = foot.x + LEFT_SHOULDER * Math.cos(phiL);
    if (topL < minTopL) {
      topR += minTopL - topL;
      topL = minTopL;
    }
  }

  const overflowR = topR + LEFT_SHOULDER - crownR;
  if (overflowR > 0) {
    phiR = PHI_MAX * smoothstep(overflowR / PHI_REACH);
    const foot = capFoot(1, phiR, track);
    const maxTopR = foot.x - LEFT_SHOULDER * Math.cos(phiR);
    if (topR > maxTopR) {
      topL -= topR - maxTopR;
      topR = maxTopR;
    }
  }

  // The tilted shoulder is shorter in x, which would pull the text onto the
  // outer edge. Grow the plateau inward by that loss and keep the label with it.
  const insetL = phiL > 0 ? LEFT_SHOULDER * (1 - Math.cos(phiL)) : 0;
  const insetR = phiR > 0 ? LEFT_SHOULDER * (1 - Math.cos(phiR)) : 0;
  topR += insetL;
  topL -= insetR;

  return { topL, topR, phiL, phiR, insetL, insetR };
}

/** Reference shoulder. phi tilts the foot onto the cap tangent; 0 is the flat scoop. */
function shoulderCmds(
  footX: number,
  footY: number,
  platX: number,
  platY: number,
  phi: number,
  dir: 1 | -1,
) {
  const xScale = Math.abs(platX - footX) / LEFT_SHOULDER;
  const yScale = Math.max(0.001, (footY - platY) / 20);
  const tangent = { x: dir * Math.cos(phi), y: -Math.sin(phi) };
  const map = (lx: number, ly: number) => ({
    x: footX + dir * lx * xScale,
    y: footY + (ly - 20) * yScale,
  });
  const h1 = 5.398 * xScale;
  const p0 = { x: footX, y: footY };
  const c1 = { x: footX + tangent.x * h1, y: footY + tangent.y * h1 };
  const c2 = map(10.012, 16.114);
  const p1 = map(10.93, 10.794);
  const c3 = map(11.916, 4.577);
  const c4 = map(17.277, 0);
  const p3 = { x: platX, y: platY };
  const fmt = (p: { x: number; y: number }) => `${p.x.toFixed(2)} ${p.y.toFixed(2)}`;
  if (dir === 1) return `C${fmt(c1)} ${fmt(c2)} ${fmt(p1)}C${fmt(c3)} ${fmt(c4)} ${fmt(p3)}`;
  return `C${fmt(c4)} ${fmt(c3)} ${fmt(p1)}C${fmt(c2)} ${fmt(c1)} ${fmt(p0)}`;
}

/** One outline: the pill, with the label shoulder grown out of its top edge. */
function shellPath(cx: number, open: number, plateauW: number, track: number) {
  const trackTop = BUMP_H;
  const trackBot = BUMP_H + HEIGHT;
  const crownL = RADIUS;
  const crownR = track - RADIUS;
  if (open < 0.012) {
    return `M${crownL} ${trackTop}H${crownR}A${RADIUS} ${RADIUS} 0 0 1 ${crownR} ${trackBot}H${crownL}A${RADIUS} ${RADIUS} 0 0 1 ${crownL} ${trackTop}Z`;
  }

  const bumpTop = BUMP_H * (1 - open);
  const { topL, topR, phiL, phiR } = shellGeom(cx, plateauW, track);
  const cmds = [`M${crownL} ${trackBot}`];

  if (phiL > 0.001) {
    const foot = capFoot(-1, phiL, track);
    cmds.push(`A${RADIUS} ${RADIUS} 0 0 1 ${foot.x.toFixed(2)} ${foot.y.toFixed(2)}`);
    cmds.push(shoulderCmds(foot.x, foot.y, topL, bumpTop, phiL, 1));
  } else {
    cmds.push(`A${RADIUS} ${RADIUS} 0 0 1 ${crownL} ${trackTop}`);
    const footL = topL - LEFT_SHOULDER;
    if (footL > crownL + 0.4) cmds.push(`H${footL.toFixed(2)}`);
    cmds.push(shoulderCmds(footL, trackTop, topL, bumpTop, 0, 1));
  }

  if (topR > topL + 0.4) cmds.push(`H${topR.toFixed(2)}`);

  if (phiR > 0.001) {
    const foot = capFoot(1, phiR, track);
    cmds.push(shoulderCmds(foot.x, foot.y, topR, bumpTop, phiR, -1));
    cmds.push(`A${RADIUS} ${RADIUS} 0 0 1 ${crownR} ${trackBot}`);
  } else {
    const footR = topR + LEFT_SHOULDER;
    cmds.push(shoulderCmds(footR, trackTop, topR, bumpTop, 0, -1));
    if (footR < crownR - 0.4) cmds.push(`H${crownR}`);
    cmds.push(`A${RADIUS} ${RADIUS} 0 0 1 ${crownR} ${trackBot}`);
  }

  cmds.push(`H${crownL}Z`);
  return cmds.join("");
}


export interface ReasoningEffortProps {
  /** The tiers, lowest → highest. Defaults to the component's own four. */
  stops?: readonly string[];
  /** Index of the current tier (controlled by the caller's model config). */
  value?: number;
  /** The model name that rides beside the tier while dragging. */
  label?: string;
  /** Accessible name of the control. */
  ariaLabel?: string;
  /** Fires once per gesture — a release or a key press, not per pixel. */
  onChange?: (index: number) => void;
  /** Text colour per stop (same order as `stops`). The tier word takes it so
   *  its strength reads at a glance; absent keeps the dim caption colour. */
  levelTones?: readonly (string | undefined)[];
  /** Changes only the rail palette; geometry, gradient and interaction stay intact. */
  accent?: "blue" | "purple";
  /** The surrounding panel has finished its opening animation. */
  effectsEnabled?: boolean;
}

export function ReasoningEffort({
  stops = DEFAULT_STOPS,
  value,
  label = DEFAULT_LABEL,
  ariaLabel = "Reasoning effort",
  onChange,
  levelTones,
  accent = "blue",
  effectsEnabled = true,
}: ReasoningEffortProps = {}) {
  // The caller owns the real tier list; index 0 is the lowest.
  const start = Math.round(value ?? 1);
  const rootRef = useRef<HTMLDivElement>(null);
  const trackRef = useRef<HTMLDivElement>(null);
  const labelRef = useRef<HTMLDivElement>(null);
  const valueRef = useRef(start);
  const displayRef = useRef(start);
  const openRef = useRef(1);
  const draggingRef = useRef(false);
  const [display, setDisplay] = useState(start);
  // The label shoulder is part of the resting shape, not a drag-only state:
  // the control always says which model and tier it stands for.
  const [open, setOpen] = useState(1);
  const [dragging, setDragging] = useState(false);
  const [theme, setTheme] = useState<"light" | "dark">("light");
  // `null` until the label has been laid out once. A guess here is what made
  // the label flash in from the wrong x and snap back.
  const [textW, setTextW] = useState<number | null>(null);
  // The control fills whatever box it is given: the geometry is linear in the
  // track length, so a wider rail keeps every proportion.
  const [track, setTrack] = useState(TRACK);
  const plateauTarget = plateauFor(textW ?? 0);
  const plateauRef = useRef(plateauTarget);
  const [plateauW, setPlateauW] = useState(plateauTarget);
  /** False until the label has been measured for the first time. */
  const plateauSettled = useRef(false);
  const revealRef = useRef(1);
  const [reveal, setReveal] = useState(1);
  const gradId = useId().replace(/:/g, "");
  const previousAccent = useRef(accent);
  const [paletteSettled, setPaletteSettled] = useState(true);
  const [starPointsVisible, setStarPointsVisible] = useState(false);
  const effectsReady = accent === "purple" && effectsEnabled && paletteSettled;
  const ambientId = useRef(0);
  const [ambientStars, setAmbientStars] = useState<
    { id: number; y: number; size: number; duration: number }[]
  >([]);

  useLayoutEffect(() => {
    if (previousAccent.current === accent) return;
    previousAccent.current = accent;
    setPaletteSettled(
      accent !== "purple" || window.matchMedia("(prefers-reduced-motion: reduce)").matches,
    );
  }, [accent]);

  useEffect(() => {
    const motion = window.matchMedia("(prefers-reduced-motion: reduce)");
    const stop = () => { if (motion.matches) setPaletteSettled(true); };
    motion.addEventListener("change", stop);
    return () => motion.removeEventListener("change", stop);
  }, []);

  useEffect(() => {
    if (!effectsReady) {
      setAmbientStars([]);
      setStarPointsVisible(false);
      return;
    }
    const motion = window.matchMedia("(prefers-reduced-motion: reduce)");
    let timer: number | undefined;
    const ready = () => !document.hidden && !motion.matches;
    const spawn = () => {
      if (!ready()) return;
      const star = {
        id: ++ambientId.current,
        y: 4 + Math.random() * 22,
        size: 2 + Math.random() * 2,
        duration: 750 + Math.random() * 550,
      };
      setAmbientStars((stars) => [...stars.slice(-1), star]);
      timer = window.setTimeout(spawn, 1700 + Math.random() * 2200);
    };
    const sync = () => {
      window.clearTimeout(timer);
      setAmbientStars([]);
      setStarPointsVisible(ready());
      if (ready()) timer = window.setTimeout(spawn, 1400 + Math.random() * 800);
    };
    motion.addEventListener("change", sync);
    document.addEventListener("visibilitychange", sync);
    sync();
    return () => {
      window.clearTimeout(timer);
      motion.removeEventListener("change", sync);
      document.removeEventListener("visibilitychange", sync);
    };
  }, [effectsReady]);

  useLayoutEffect(() => {
    const el = rootRef.current;
    if (!el) return;
    const measure = () => {
      const width = el.clientWidth;
      if (width > 0) setTrack(width);
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  useLayoutEffect(() => {
    const el = rootRef.current;
    if (!el) return;
    const read = () => {
      const marked = el.parentElement?.closest("[data-theme]");
      if (marked) {
        setTheme(marked.getAttribute("data-theme") === "dark" ? "dark" : "light");
        return;
      }
      if (el.closest(".dark")) {
        setTheme("dark");
        return;
      }
      setTheme(window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light");
    };
    read();
    const marked = el.parentElement?.closest("[data-theme]");
    const obs = new MutationObserver(read);
    if (marked) obs.observe(marked, { attributes: true, attributeFilter: ["data-theme"] });
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    media.addEventListener("change", read);
    return () => {
      obs.disconnect();
      media.removeEventListener("change", read);
    };
  }, []);

  useEffect(() => {
    const reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    const toOpen = 1;
    const fromOpen = openRef.current;
    const fromValue = displayRef.current;
    const targetValue = dragging ? fromValue : Math.round(valueRef.current);
    if (reduce) {
      openRef.current = toOpen;
      setOpen(toOpen);
      if (!dragging) {
        valueRef.current = targetValue;
        displayRef.current = targetValue;
        setDisplay(targetValue);
      }
      return;
    }
    const start = performance.now();
    let raf = 0;
    const step = (now: number) => {
      const t = Math.min(1, (now - start) / 220);
      const e = easeOut(t);
      const nextOpen = fromOpen + (toOpen - fromOpen) * e;
      openRef.current = nextOpen;
      setOpen(nextOpen);
      if (!dragging) {
        const nextValue = fromValue + (targetValue - fromValue) * e;
        displayRef.current = nextValue;
        setDisplay(nextValue);
        if (t === 1) {
          valueRef.current = targetValue;
          displayRef.current = targetValue;
        }
      }
      if (t < 1) raf = requestAnimationFrame(step);
    };
    raf = requestAnimationFrame(step);
    return () => cancelAnimationFrame(raf);
  }, [dragging]);

  // Layout effect, not effect: the plateau decides where the label sits, and a
  // post-paint correction would show the label at its first guess and then snap
  // it sideways.
  useLayoutEffect(() => {
    const reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    const from = plateauRef.current;
    const to = plateauTarget;
    // The first measurement lands once the label has been laid out; tweening
    // into it would slide the label sideways as the control appears.
    if (!plateauSettled.current) {
      plateauSettled.current = true;
      plateauRef.current = to;
      setPlateauW(to);
      return;
    }
    if (reduce || Math.abs(to - from) < 0.3) {
      plateauRef.current = to;
      setPlateauW(to);
      return;
    }
    const start = performance.now();
    let raf = 0;
    const step = (now: number) => {
      const t = Math.min(1, (now - start) / 90);
      const next = from + (to - from) * easeOut(t);
      plateauRef.current = next;
      setPlateauW(next);
      if (t < 1) raf = requestAnimationFrame(step);
    };
    raf = requestAnimationFrame(step);
    return () => cancelAnimationFrame(raf);
  }, [plateauTarget]);

  useEffect(() => {
    const reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    // Always shown: the label is the control's own caption, not a tooltip.
    const to = 1;
    const from = revealRef.current;
    if (reduce) {
      revealRef.current = to;
      setReveal(to);
      return;
    }
    const delay = dragging ? 85 : 0;
    const dur = dragging ? 132 : 90;
    const start = performance.now();
    let raf = 0;
    const step = (now: number) => {
      const elapsed = now - start - delay;
      if (elapsed < 0) {
        raf = requestAnimationFrame(step);
        return;
      }
      const t = Math.min(1, elapsed / dur);
      const next = from + (to - from) * easeOut(t);
      revealRef.current = next;
      setReveal(next);
      if (t < 1) raf = requestAnimationFrame(step);
    };
    raf = requestAnimationFrame(step);
    return () => cancelAnimationFrame(raf);
  }, [dragging]);

  function commit(next: number) {
    valueRef.current = next;
    displayRef.current = next;
    setDisplay(next);
  }

  /** One call per gesture: releasing the pointer, or a key press. */
  function report() {
    onChange?.(Math.round(valueRef.current));
  }

  function valueFrom(clientX: number) {
    const rect = trackRef.current?.getBoundingClientRect();
    if (!rect) return valueRef.current;
    const t = (clientX - rect.left - RADIUS) / (rect.width - RADIUS * 2);
    return clamp(t * (stops.length - 1), 0, stops.length - 1);
  }

  function onPointerDown(event: ReactPointerEvent<HTMLDivElement>) {
    if (event.button !== 0) return;
    draggingRef.current = true;
    try {
      event.currentTarget.setPointerCapture(event.pointerId);
    } catch {
      /* Pointer capture needs a trusted event. */
    }
    setDragging(true);
    commit(valueFrom(event.clientX));
  }

  function onPointerMove(event: ReactPointerEvent<HTMLDivElement>) {
    if (!draggingRef.current) return;
    commit(valueFrom(event.clientX));
  }

  function onPointerUp(event: ReactPointerEvent<HTMLDivElement>) {
    if (!draggingRef.current) return;
    draggingRef.current = false;
    try {
      if (event.currentTarget.hasPointerCapture(event.pointerId)) {
        event.currentTarget.releasePointerCapture(event.pointerId);
      }
    } catch {
      /* Ignore when the event was not trusted. */
    }
    setDragging(false);
    report();
  }

  function onKeyDown(event: React.KeyboardEvent<HTMLDivElement>) {
    const max = stops.length - 1;
    const current = Math.round(valueRef.current);
    let next = current;
    if (event.key === "ArrowRight" || event.key === "ArrowUp") next = Math.min(max, current + 1);
    else if (event.key === "ArrowLeft" || event.key === "ArrowDown") next = Math.max(0, current - 1);
    else if (event.key === "Home") next = 0;
    else if (event.key === "End") next = max;
    else return;
    event.preventDefault();
    commit(next);
    onChange?.(next);
  }

  useEffect(() => {
    if (value == null) return;
    const next = Math.round(value);
    if (next === Math.round(valueRef.current)) return;
    valueRef.current = next;
    displayRef.current = next;
    setDisplay(next);
  }, [value]);

  const center = centerFor(display, stops.length, track);
  // Only dragging widens the thumb: the resting shape no longer tracks `open`,
  // which is pinned for the label shoulder.
  const thumbW = REST_W + (DRAG_W - REST_W) * (dragging ? 1 : 0);
  const thumbH = REST_H;
  const outerPad = EDGE_PAD;
  let thumbLeft = center - thumbW / 2;
  let thumbRight = thumbLeft + thumbW;
  if (thumbLeft < outerPad) {
    thumbLeft = outerPad;
    thumbRight = thumbLeft + thumbW;
  }
  if (thumbRight > track - outerPad) {
    thumbRight = track - outerPad;
    thumbLeft = thumbRight - thumbW;
  }
  const fillW = Math.min(track, thumbRight + EDGE_PAD);
  const active = Math.round(display);
  const level = stops[active];
  // The tier text re-enters on a change, never on the first paint: entering the
  // control must not look like an animation. Once any change has happened, a
  // remount is a change by definition.
  const firstLevel = useRef(level);
  const [animateLabel, setAnimateLabel] = useState(false);
  useEffect(() => {
    if (level !== firstLevel.current) setAnimateLabel(true);
  }, [level]);
  const tickHeights = stops.map((_, i) =>
    4.8 + (i / Math.max(1, stops.length - 1)) * 4.8,
  );
  const plateau = shellGeom(center, plateauW, track);
  const labelX =
    (plateau.topL + plateau.topR) / 2 + (plateau.insetL - plateau.insetR) / 2;

  useLayoutEffect(() => {
    const w = labelRef.current?.getBoundingClientRect().width ?? 0;
    if (w <= 0) return;
    if (textW === null) {
      // First measurement: place the shoulder and the label in the SAME
      // pre-paint pass, so no frame is ever drawn at the placeholder width.
      const target = plateauFor(w);
      plateauRef.current = target;
      plateauSettled.current = true;
      setPlateauW(target);
      setTextW(w);
      return;
    }
    if (Math.abs(w - textW) > 0.4) setTextW(w);
  }, [level, theme, textW]);

  return (
    <div
      ref={rootRef}
      className={s.root}
      data-theme={theme}
      data-accent={accent}
      data-dragging={dragging ? "true" : "false"}
    >
      <svg
        className={s.shell}
        viewBox={`0 0 ${track} ${BUMP_H + HEIGHT}`}
        aria-hidden
      >
        <defs>
          <linearGradient id={gradId} x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="var(--ss-shell)" />
            <stop offset="1" stopColor="var(--ss-shell-2)" />
          </linearGradient>
        </defs>
        <path
          d={shellPath(center, open, plateauW, track)}
          fill={`url(#${gradId})`}
          stroke="var(--ss-line)"
          strokeWidth="0.5"
          strokeLinejoin="round"
          vectorEffect="non-scaling-stroke"
        />
      </svg>
      <div
        ref={labelRef}
        className={s.label}
        style={{
          left: labelX,
          opacity: textW === null ? 0 : reveal,
          filter: reveal > 0.98 ? undefined : `blur(${((1 - reveal) * 3).toFixed(2)}px)`,
          transform: `translate(-50%, ${3 * (1 - open)}px)`,
        }}
      >
        {/* Only the tier re-enters on a move — the model name is the control's
            caption and must hold still while the thumb travels. */}
        <span className={s.swap}>
          <span className={s.current}>{label}</span>
          <span key={level} className={s.swap + (animateLabel ? " " + s.enter : "")}>
            <span className={s.next} style={levelTones?.[active] ? { color: levelTones[active] } : undefined}>
              {level}
            </span>
          </span>
        </span>
      </div>
      <div
        ref={trackRef}
        className={s.track}
        role="slider"
        tabIndex={0}
        aria-label={ariaLabel}
        aria-valuemin={0}
        aria-valuemax={stops.length - 1}
        aria-valuenow={active}
        aria-valuetext={`${label} ${level}`}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={onPointerUp}
        onKeyDown={onKeyDown}
      >
        <div className={s.fill} style={{ width: fillW }}>
          <div
            className={s.fastFill}
            data-fast-fill=""
            aria-hidden="true"
            onTransitionEnd={(event) => {
              if (event.target === event.currentTarget && event.propertyName === "transform" && accent === "purple") {
                setPaletteSettled(true);
              }
            }}
          >
            {accent === "purple" && !paletteSettled && effectsEnabled && (
              <span className={s.frontSparkles} data-fast-front="" aria-hidden="true">
                {FAST_FRONT_SPARKLES.map((star, index) => (
                  <span
                    key={index}
                    className={s.frontStar}
                    style={{
                      left: star.x, top: star.y, width: star.size, height: star.size,
                      animationDelay: `${star.delay}ms`,
                    }}
                  />
                ))}
              </span>
            )}
          </div>
        </div>
        <div
          className={s.railEffects}
          data-fast-background=""
          style={{ clipPath: `inset(0 ${Math.max(0, track - fillW)}px 0 0 round 16px)` }}
          aria-hidden="true"
        >
          {effectsReady && starPointsVisible && (
            <span className={s.fastPoints} data-fast-points="" aria-hidden="true">
              {FAST_POINTS.map((point, index) => (
                <span
                  key={index}
                  className={s.fastPoint}
                  style={{
                    left: `${point.x}%`, top: `${point.y}%`,
                    width: point.size, height: point.size,
                    animationDuration: `${point.duration}s`, animationDelay: `${point.delay}s`,
                  }}
                />
              ))}
            </span>
          )}
          {effectsReady && ambientStars.map((star) => (
            <span
              key={star.id}
              className={s.ambientPass}
              data-fast-ambient=""
              aria-hidden="true"
              style={{
                top: star.y, height: star.size,
                animationDuration: `${star.duration}ms`,
              }}
              onAnimationEnd={(event) => {
                if (event.target === event.currentTarget) {
                  setAmbientStars((stars) => stars.filter((item) => item.id !== star.id));
                }
              }}
            >
              <span className={s.ambientStar} style={{ width: star.size }} />
            </span>
          ))}
        </div>
        <div
          className={s.thumb}
          style={{
            left: thumbLeft,
            width: thumbW,
            height: thumbH,
            marginTop: -thumbH / 2,
          }}
        />
        {stops.map((label, index) => {
          const x = centerFor(index, stops.length, track);
          const onThumb = x >= thumbLeft + 0.5 && x <= thumbRight - 0.5;
          return (
            <span
              key={label}
              className={s.tick}
              data-on-thumb={onThumb ? "true" : "false"}
              data-active={index === active && !onThumb ? "true" : "false"}
              style={{ left: x, height: tickHeights[index] }}
            />
          );
        })}
      </div>
    </div>
  );
}
