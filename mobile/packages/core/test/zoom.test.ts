import { describe, expect, it } from 'vitest';
import {
  DOUBLE_TAP_MS,
  DOUBLE_TAP_SCALE,
  MAX_SCALE,
  clampView,
  doubleTap,
  isTap,
  pinch,
  tapKind,
} from '../src/zoom';

const W = 400;
const H = 800;
const fit = { s: 1, x: 0, y: 0 };

describe('image zoom maths', () => {
  it('does not pan at scale 1 and stops at the edges when zoomed', () => {
    expect(clampView({ s: 1, x: 50, y: -50 }, W, H)).toEqual({ s: 1, x: 0, y: 0 });
    expect(clampView({ s: 2, x: 999, y: -999 }, W, H)).toEqual({ s: 2, x: 200, y: -400 });
    expect(clampView({ s: 99, x: 0, y: 0 }, W, H).s).toBe(MAX_SCALE);
    expect(clampView({ s: 0.2, x: 0, y: 0 }, W, H).s).toBe(1);
  });

  it('pinching apart scales by the ratio of the distances and together shrinks back', () => {
    const c = { x: 0, y: 0 };
    expect(pinch(fit, 100, 200, c, c, W, H).s).toBe(2);
    expect(pinch(fit, 100, 350, c, c, W, H).s).toBe(3.5);
    expect(pinch({ s: 2, x: 0, y: 0 }, 200, 100, c, c, W, H).s).toBe(1);
    expect(pinch(fit, 100, 5000, c, c, W, H).s).toBe(MAX_SCALE);
    expect(pinch(fit, 0, 100, c, c, W, H)).toEqual(fit);
  });

  it('keeps the point between the fingers under them', () => {
    const at = { x: 100, y: 100 };
    const v = pinch(fit, 100, 200, at, at, W, H);
    // the image point that was under (100,100) is still there: screen = s * p + t
    expect(v.s * 100 + v.x).toBeCloseTo(100);
    expect(v.s * 100 + v.y).toBeCloseTo(100);
    // fingers sliding while pinching drag the image with them
    const moved = pinch(fit, 100, 200, at, { x: 130, y: 100 }, W, H);
    expect(moved.s * 100 + moved.x).toBeCloseTo(130);
  });

  it('double tap zooms in on the tapped point and back out on the next', () => {
    const at = { x: 100, y: -200 };
    const zoomed = doubleTap(fit, at, W, H);
    expect(zoomed.s).toBe(DOUBLE_TAP_SCALE);
    expect(zoomed.s * at.x + zoomed.x).toBeCloseTo(at.x);
    expect(zoomed.s * at.y + zoomed.y).toBeCloseTo(at.y);
    expect(doubleTap(zoomed, at, W, H)).toEqual(fit);
    // a tap at the edge cannot pan the image past it
    const edge = doubleTap(fit, { x: 200, y: 400 }, W, H);
    expect(Math.abs(edge.x)).toBeLessThanOrEqual((W * (edge.s - 1)) / 2);
  });

  it('tells the second quick, near tap of a double tap from two separate taps', () => {
    const a = { at: 1000, x: 10, y: 10 };
    expect(tapKind(null, a)).toBe('single');
    expect(tapKind(a, { at: 1000 + DOUBLE_TAP_MS - 10, x: 20, y: 15 })).toBe('double');
    expect(tapKind(a, { at: 1000 + DOUBLE_TAP_MS + 10, x: 10, y: 10 })).toBe('single');
    expect(tapKind(a, { at: 1100, x: 300, y: 300 })).toBe('single');
  });

  it('a tap is short and does not travel', () => {
    expect(isTap(120, 3)).toBe(true);
    expect(isTap(400, 3)).toBe(false);
    expect(isTap(100, 30)).toBe(false);
  });
});
