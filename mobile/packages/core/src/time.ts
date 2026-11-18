/** The store keeps some times in seconds (`last_reply_at`, mentions) and the rest in ms. */
export const toMs = (value: number | null | undefined): number | null => {
  if (value === null || value === undefined || !Number.isFinite(value)) return null;
  return value < 100_000_000_000 ? value * 1000 : value;
};

export function span(ms: number): string {
  const s = Math.max(0, Math.round(ms / 1000));
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  if (h < 48) return `${h}h`;
  return `${Math.floor(h / 24)}d`;
}

export function since(at: number | null | undefined, now = Date.now()): string {
  const ms = toMs(at);
  if (ms === null) return '';
  return span(now - ms);
}

export function hhmm(at: number | null | undefined): string {
  const ms = toMs(at);
  if (ms === null) return '';
  const d = new Date(ms);
  return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
}

export function size(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  return `${(bytes / 1024 / 1024 / 1024).toFixed(1)} GB`;
}

/** "just now", "4m ago", "2h ago", "3d ago" (long: "4 minutes ago"); Japanese "4分前". */
export function ago(
  at: number | null | undefined,
  now = Date.now(),
  lang: 'en' | 'ja' = 'en',
  long = false,
): string {
  const ms = toMs(at);
  if (ms === null) return '';
  const s = Math.max(0, Math.round((now - ms) / 1000));
  if (s < 45) return lang === 'ja' ? 'たった今' : 'just now';
  const m = Math.max(1, Math.round(s / 60));
  const unit = (n: number, short: string, one: string, many: string, ja: string) =>
    lang === 'ja' ? `${n}${ja}前` : long ? `${n} ${n === 1 ? one : many} ago` : `${n}${short} ago`;
  if (m < 60) return unit(m, 'm', 'minute', 'minutes', '分');
  const h = Math.floor(m / 60);
  if (h < 24) return unit(h, 'h', 'hour', 'hours', '時間');
  const d = Math.floor(h / 24);
  return unit(d, 'd', 'day', 'days', '日');
}

/** The local calendar day of a time, as a number that compares and differs by day. */
export function dayNumber(at: number | null | undefined): number | null {
  const ms = toMs(at);
  if (ms === null) return null;
  const d = new Date(ms);
  return Math.floor(Date.UTC(d.getFullYear(), d.getMonth(), d.getDate()) / 86_400_000);
}

export type DayLabel = { kind: 'today' } | { kind: 'yesterday' } | { kind: 'date'; at: number };

/** What a day divider says: Today, Yesterday, or the date (the app formats it in the person's language). */
export function dayLabel(at: number, now = Date.now()): DayLabel {
  const a = dayNumber(at);
  const n = dayNumber(now)!;
  if (a === n) return { kind: 'today' };
  if (a === n - 1) return { kind: 'yesterday' };
  return { kind: 'date', at: toMs(at)! };
}
