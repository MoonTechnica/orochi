import type { Member, ThreadRow, ThreadStatus } from '@orochi/protocol';
import { t, type Lang } from './i18n';

export const STATE_LABELS: Record<ThreadStatus, string> = {
  needs_you: 'Needs you',
  working: 'Working',
  background: 'In the background',
  queued: 'Queued',
  held: 'Waiting for a limit to lift',
  interrupted: 'Interrupted',
  failed: 'Failed',
  unread: 'Unread',
  idle: 'Idle',
};

export type Tone = 'live' | 'ask' | 'ok' | 'warn' | 'danger' | 'muted';

export const STATUS_TONE: Record<ThreadStatus, Tone> = {
  needs_you: 'ask',
  working: 'live',
  background: 'live',
  queued: 'muted',
  held: 'warn',
  interrupted: 'warn',
  failed: 'danger',
  unread: 'ok',
  idle: 'muted',
};

/** Single-glyph marks; the app draws them, tests read them. */
export const STATUS_MARK: Record<ThreadStatus, string> = {
  needs_you: '●',
  working: '◐',
  background: '◔',
  queued: '◷',
  held: '◷',
  interrupted: '‖',
  failed: '!',
  unread: '•',
  idle: '○',
};

export const needsYou = (th: ThreadRow) =>
  th.status === 'needs_you' || th.asking > 0 || (th.mentions ?? 0) > 0;

export const NEEDS: Record<string, string> = {
  asking: 'asking you',
  mention: 'mentioned you',
  worktree: 'worktree open',
  trial: 'trial not applied',
};

export interface MemberState {
  tone: Tone;
  label: string;
}

/** Where a member stands, as its bar says it (desktop `memberState`). */
export function memberState(m: Member | undefined | null, lang: Lang = 'en'): MemberState {
  if (!m) return { tone: 'muted', label: t('done', lang) };
  const working = m.live || ['working', 'choosing', 'starting'].includes(m.state);
  if (m.state === 'asking') return { tone: 'ask', label: t('asking', lang) };
  if (working) {
    const doing = (m.doing || '').split('\n')[0];
    return { tone: 'live', label: doing ? `${t('working', lang)} — ${doing}` : t('working', lang) };
  }
  if (m.state === 'failed') return { tone: 'danger', label: t('failed', lang) };
  if (['interrupted', 'cancelled', 'stopped'].includes(m.state))
    return { tone: 'warn', label: t('stopped', lang) };
  const ws = m.workspace_state;
  if (ws === 'applied' || ws === 'merged') return { tone: 'ok', label: t('applied', lang) };
  if (ws === 'patch') return { tone: 'warn', label: t('patch kept', lang) };
  if (ws === 'pr') return { tone: 'ok', label: t('pull request', lang) };
  if (m.trial_state === 'clean' && ws === 'open')
    return { tone: 'ask', label: t('trial clean', lang) };
  if (m.reported) return { tone: 'muted', label: t('reported', lang) };
  return { tone: 'muted', label: t('done', lang) };
}

export const needsLabel = (needs: string | null | undefined, lang: Lang = 'en') =>
  needs ? t(NEEDS[needs] ?? needs, lang) : '';

export const BUCKETS: [string, (th: ThreadRow) => boolean][] = [
  ['Needs you', (th) => th.status === 'needs_you'],
  ['Working', (th) => th.status === 'working' || th.status === 'queued'],
  ['In the background', (th) => th.status === 'background'],
  ['Done', () => true],
];

export function bucketize(threads: ThreadRow[]): { name: string; threads: ThreadRow[] }[] {
  const placed = new Set<string>();
  return BUCKETS.map(([name, fits]) => {
    const rows = threads.filter((th) => !placed.has(th.id) && fits(th));
    rows.forEach((th) => placed.add(th.id));
    return { name, threads: rows };
  });
}

export const AT: Record<string, string> = {
  choosing: 'choosing an agent…',
  starting: 'starting up…',
  working: 'working…',
  asking: 'waiting for your answer',
  done: 'done',
  failed: 'failed',
};
