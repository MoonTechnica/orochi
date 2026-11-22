/** A moment in unix seconds, as a list says it: today's clock, or the day and the clock. */
export function whenOf(seconds: number | null | undefined, now = Date.now()): string {
  if (!seconds) return '—';
  const date = new Date(seconds * 1000);
  const clock = date.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' });
  if (date.toDateString() === new Date(now).toDateString()) return clock;
  return `${date.toLocaleDateString(undefined, { month: 'short', day: 'numeric' })} ${clock}`;
}

// ---- push ------------------------------------------------------------------------------

/**
 * A random id for this phone's push registration: 32 hex characters, which is all it is. It is
 * what a notice is addressed to, and names nobody — never an email, a name or an Apple ID.
 */
export function newPushId(random: (bytes: Uint8Array) => void): string {
  const bytes = new Uint8Array(16);
  random(bytes);
  return Array.from(bytes, (b) => b.toString(16).padStart(2, '0')).join('');
}

/** The desktop accepts 16 to 64 letters, digits, `-` and `_`. */
export const validPushId = (id: string): boolean => /^[A-Za-z0-9_-]{16,64}$/.test(id);

export interface PushTap {
  machine: string | null;
  thread: string | null;
  notice: number | null;
}

/**
 * What a tapped push carries. `data` is only ids (the desktop never puts content in it), and
 * is read defensively: a push is input from outside the app.
 */
export function pushTap(data: unknown): PushTap | null {
  if (!data || typeof data !== 'object') return null;
  const d = data as Record<string, unknown>;
  const str = (v: unknown) => (typeof v === 'string' && v.length > 0 && v.length < 200 ? v : null);
  const tap = {
    machine: str(d.machine),
    thread: str(d.thread),
    notice: typeof d.notice === 'number' && Number.isFinite(d.notice) ? d.notice : null,
  };
  return tap.thread || tap.notice !== null ? tap : null;
}

const DAYS_JA: Record<string, string> = {
  mon: '月',
  tue: '火',
  wed: '水',
  thu: '木',
  fri: '金',
  sat: '土',
  sun: '日',
};
const UNIT_JA: Record<string, string> = { m: '分', h: '時間', d: '日' };

/**
 * A schedule as the person reads it. The schedule is written in Orochi's English grammar
 * (`daily at 08:00`, `every 2h from 09:00 to 19:00`); in Japanese it is said in Japanese, and
 * anything not recognised (a cron line) is shown as written.
 */
export function scheduleWords(text: string | null | undefined, lang: 'en' | 'ja'): string {
  const s = (text ?? '').trim();
  if (lang !== 'ja' || !s) return s;
  const times = (list: string) => list.split(/\s*,\s*/).join('・');
  let m: RegExpExecArray | null;
  if ((m = /^daily at (.+)$/i.exec(s))) return `毎日 ${times(m[1]!)}`;
  if ((m = /^weekdays at (.+)$/i.exec(s))) return `平日 ${times(m[1]!)}`;
  if ((m = /^weekends at (.+)$/i.exec(s))) return `週末 ${times(m[1]!)}`;
  if (
    (m =
      /^every ((?:mon|tue|wed|thu|fri|sat|sun)(?:\s*,\s*(?:mon|tue|wed|thu|fri|sat|sun))*) at (.+)$/i.exec(
        s,
      ))
  )
    return `毎週 ${m[1]!
      .split(/\s*,\s*/)
      .map((d) => DAYS_JA[d.toLowerCase()] ?? d)
      .join('・')} ${times(m[2]!)}`;
  if ((m = /^every (\d+)\s*([mhd]) from (\S+) to (\S+)$/i.exec(s)))
    return `${m[3]}〜${m[4]} の間 ${m[1]}${UNIT_JA[m[2]!.toLowerCase()]}ごと`;
  if ((m = /^every (\d+)\s*([mhd])$/i.exec(s))) return `${m[1]}${UNIT_JA[m[2]!.toLowerCase()]}ごと`;
  if ((m = /^in (\d+)\s*([mhd])$/i.exec(s)))
    return `${m[1]}${UNIT_JA[m[2]!.toLowerCase()]}後に 1 回`;
  if ((m = /^on (\d{4}-\d{2}-\d{2}) at (\S+)$/i.exec(s))) return `${m[1]} ${m[2]} に 1 回`;
  if ((m = /^at (\S+)$/i.exec(s))) return `${m[1]} に 1 回`;
  return s;
}
