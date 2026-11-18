/** The maths of the image viewer's gestures, apart from the views that feel them. */

export interface View2D {
  /** Scale, and the translation of the image's centre from the frame's centre, in points. */
  s: number;
  x: number;
  y: number;
}

export const MIN_SCALE = 1;
export const MAX_SCALE = 6;
/** Where a double tap goes when the image is not zoomed. */
export const DOUBLE_TAP_SCALE = 2.5;

const clamp = (v: number, lo: number, hi: number) => Math.max(lo, Math.min(hi, v));

/** Keeps the image covering the frame: no pan past an edge, none at all at scale 1. */
export function clampView(v: View2D, width: number, height: number): View2D {
  const s = clamp(v.s, MIN_SCALE, MAX_SCALE);
  const lx = (width * (s - 1)) / 2;
  const ly = (height * (s - 1)) / 2;
  return { s, x: clamp(v.x, -lx, lx) + 0, y: clamp(v.y, -ly, ly) + 0 };
}

export const distance = (a: { x: number; y: number }, b: { x: number; y: number }) =>
  Math.hypot(a.x - b.x, a.y - b.y);

/**
 * A pinch: the scale at the start times how far the fingers have moved apart, and the point
 * between the fingers stays where it was (`start` and `now` are the fingers' midpoints relative to the
 * frame's centre, `from` the view when the pinch began).
 */
export function pinch(
  from: View2D,
  startDist: number,
  dist: number,
  start: { x: number; y: number },
  now: { x: number; y: number },
  width: number,
  height: number,
): View2D {
  if (startDist <= 0 || dist <= 0) return clampView(from, width, height);
  const s = clamp((from.s * dist) / startDist, MIN_SCALE, MAX_SCALE);
  // the image point under the starting midpoint, then put under the current one
  const px = (start.x - from.x) / from.s;
  const py = (start.y - from.y) / from.s;
  return clampView({ s, x: now.x - s * px, y: now.y - s * py }, width, height);
}

/** Double tap: back to fit when zoomed, otherwise in on the tapped point (relative to the centre). */
export function doubleTap(
  from: View2D,
  at: { x: number; y: number },
  width: number,
  height: number,
): View2D {
  if (from.s > 1.02) return { s: 1, x: 0, y: 0 };
  const s = DOUBLE_TAP_SCALE;
  const px = (at.x - from.x) / from.s;
  const py = (at.y - from.y) / from.s;
  return clampView({ s, x: at.x - s * px, y: at.y - s * py }, width, height);
}

export interface Tap {
  at: number;
  x: number;
  y: number;
}

/** Generous on purpose: a simulator's (and a slow thumb's) second tap comes later than 300 ms. */
export const DOUBLE_TAP_MS = 450;
export const DOUBLE_TAP_SLOP = 48;

/** A tap that follows another one closely and nearby is the second of a double tap. */
export function tapKind(last: Tap | null, tap: Tap): 'single' | 'double' {
  return last && tap.at - last.at <= DOUBLE_TAP_MS && distance(last, tap) <= DOUBLE_TAP_SLOP
    ? 'double'
    : 'single';
}

/** A touch is a tap when it was short and did not travel. */
export const isTap = (ms: number, travelled: number) => ms < 250 && travelled < 10;
