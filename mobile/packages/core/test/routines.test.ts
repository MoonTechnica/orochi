import { describe, expect, it } from 'vitest';
import { scheduleWords, newPushId, pushTap, validPushId, whenOf } from '../src/routines';

describe('the time a list shows', () => {
  it('is a clock today and a date with the clock another day', () => {
    const noon = new Date(2026, 10, 21, 12, 0).getTime();
    const today = Math.floor(new Date(2026, 10, 21, 18, 30).getTime() / 1000);
    const later = Math.floor(new Date(2026, 10, 23, 9, 0).getTime() / 1000);
    expect(whenOf(today, noon)).not.toMatch(/Nov/);
    expect(whenOf(later, noon)).toMatch(/23/);
    expect(whenOf(null, noon)).toBe('—');
    expect(whenOf(0, noon)).toBe('—');
  });
});

describe('the id notices for this phone are addressed to', () => {
  it('is random hex the desktop accepts, and names nobody', () => {
    const fill = (bytes: Uint8Array) => bytes.forEach((_, i) => (bytes[i] = (i * 17 + 3) % 256));
    const id = newPushId(fill);
    expect(id).toMatch(/^[0-9a-f]{32}$/);
    expect(validPushId(id)).toBe(true);
    for (const bad of [
      'short',
      'has spaces in it that is long enough',
      'me@example.com-aaaaaaaa',
      '',
    ])
      expect(validPushId(bad)).toBe(false);
  });
});

describe('a tapped push', () => {
  it('is read for the ids it carries and nothing else', () => {
    expect(pushTap({ machine: 'abc', thread: 't1', notice: 7, kind: 'ask' })).toEqual({
      machine: 'abc',
      thread: 't1',
      notice: 7,
    });
    expect(pushTap({ notice: 3 })).toEqual({ machine: null, thread: null, notice: 3 });
    expect(pushTap({ machine: 'abc' })).toBeNull();
    expect(pushTap(null)).toBeNull();
    expect(pushTap('thread')).toBeNull();
    expect(pushTap({ thread: 'x'.repeat(500) })).toBeNull();
    expect(pushTap({ thread: 5, notice: 'a' })).toBeNull();
  });
});

describe('a schedule in the person’s language', () => {
  it('says each form of the grammar in Japanese and leaves the rest as written', () => {
    const ja = (s: string) => scheduleWords(s, 'ja');
    expect(ja('daily at 07:30')).toBe('毎日 07:30');
    expect(ja('weekdays at 08:00, 12:00')).toBe('平日 08:00・12:00');
    expect(ja('every mon,thu at 10:00')).toBe('毎週 月・木 10:00');
    expect(ja('every 2h from 09:00 to 19:00')).toBe('09:00〜19:00 の間 2時間ごと');
    expect(ja('every 15m')).toBe('15分ごと');
    expect(ja('in 90m')).toBe('90分後に 1 回');
    expect(ja('on 2026-11-24 at 09:30')).toBe('2026-11-24 09:30 に 1 回');
    expect(ja('at 15:00')).toBe('15:00 に 1 回');
    expect(ja('cron 0 9 * * 1')).toBe('cron 0 9 * * 1');
    expect(scheduleWords('daily at 07:30', 'en')).toBe('daily at 07:30');
  });
});
