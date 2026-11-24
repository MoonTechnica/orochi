import React, { useState } from 'react';
import { ago, needsYou, STATE_LABELS, toMs } from '@orochi/core';
import type { ThreadRow } from '@orochi/protocol';
import { useAttention } from '../../src/state/attention';
import { useOpenThread } from '../../src/state/open';
import { usePrefs } from '../../src/state/prefs';
import { ConversationInbox } from '../../src/ui/ConversationInbox';

const active = (thread: ThreadRow) =>
  thread.status === 'working' || thread.status === 'queued' || thread.status === 'background';
const priority = (thread: ThreadRow) =>
  needsYou(thread) ? 0 : active(thread) ? 1 : thread.unread ? 2 : 3;

export default function AllConversations() {
  const { t, lang } = usePrefs();
  const { snapshots, refreshAll } = useAttention();
  const open = useOpenThread();
  const [filter, setFilter] = useState<'all' | 'needs' | 'unread'>('all');
  const filters = [
    { key: 'all', label: t('All') },
    { key: 'needs', label: t('Needs you') },
    { key: 'unread', label: t('Unread') },
  ] as const;

  return (
    <ConversationInbox
      title={t('Conversations')}
      refresh={refreshAll}
      filters={filters.map(({ key, label }) => ({
        label,
        selected: key === filter,
        onPress: () => setFilter(key),
      }))}
      sections={snapshots.map((workspace) => {
        const rows =
          workspace.state === 'online'
            ? workspace.projects.flatMap((project) =>
                project.missing ? [] : project.threads.filter((thread) => !thread.archived),
              )
            : [];
        const stat = (key: string, count: number) => t(key).replace('{n}', String(count));
        return {
          key: workspace.key,
          title: workspace.name,
          state: workspace.state,
          summary: rows.length
            ? [
                stat('{n} awaiting your response', rows.filter(needsYou).length),
                stat('{n} active', rows.filter(active).length),
                stat('{n} unread', rows.filter((thread) => thread.unread > 0).length),
              ].join(' · ')
            : undefined,
          empty: filter === 'all' ? t('No conversations') : t('Nothing here.'),
          data: rows
            .filter(
              (thread) =>
                filter === 'all' || (filter === 'needs' ? needsYou(thread) : thread.unread > 0),
            )
            .sort(
              (a, b) =>
                priority(a) - priority(b) || (toMs(b.updated_at) ?? 0) - (toMs(a.updated_at) ?? 0),
            )
            .map((thread) => ({
              key: thread.id,
              title: thread.title || t('New conversation'),
              status: t(needsYou(thread) ? 'Needs you' : STATE_LABELS[thread.status]),
              needsReply: needsYou(thread),
              summary: [thread.project, ago(thread.updated_at, Date.now(), lang)]
                .filter(Boolean)
                .join(' · '),
              onPress: () => void open(workspace.key, thread.id),
            })),
        };
      })}
    />
  );
}
