import React from 'react';
import { Text, View } from 'react-native';
import { toMs } from '@orochi/core';
import type { AgentRow } from '@orochi/protocol';
import { usePrefs } from '../state/prefs';

const BUCKET_LABELS: Record<string, string> = {
  five_hour: '5-hour limit',
  seven_day: '7-day limit',
  primary: 'Primary limit',
  secondary: 'Secondary limit',
  spend_limit: 'Spending limit',
};

function quotaMeters(rows: AgentRow[], translate: (key: string) => string) {
  const meters = new Map<
    string,
    { label: string; remaining: number; reset: number | null; estimated: boolean }
  >();
  const withWindows = new Set<string>();
  const now = Date.now();
  const valid = (remaining: number, reset: number | null) =>
    Number.isFinite(remaining) &&
    remaining >= 0 &&
    remaining <= 1 &&
    (reset === null || reset > now);
  for (const row of rows) {
    for (const [bucket, remaining, resetAt] of row.windows ?? []) {
      if (bucket.endsWith('/credits')) continue;
      const reset = toMs(resetAt);
      if (!valid(remaining, reset)) continue;
      withWindows.add(row.agent);
      meters.set(JSON.stringify([row.agent, bucket]), {
        label: `${row.agent} · ${translate(BUCKET_LABELS[bucket] ?? BUCKET_LABELS[bucket.split('/').pop() ?? ''] ?? bucket.replace(/_/g, ' '))}`,
        remaining,
        reset,
        estimated: false,
      });
    }
  }
  const accountRows = new Set(rows.filter((row) => row.model === '*').map((row) => row.agent));
  for (const row of rows) {
    const remaining = row.quota_estimate;
    const reset = toMs(row.reset_at);
    if (withWindows.has(row.agent) || remaining === null || !valid(remaining, reset)) continue;
    if (row.model !== '*' && accountRows.has(row.agent)) continue;
    const label = row.model === '*' ? row.agent : `${row.agent} · ${row.model}`;
    meters.set(JSON.stringify([row.agent, row.model]), {
      label,
      remaining,
      reset,
      estimated: true,
    });
  }
  return [...meters.entries()];
}

export function AgentQuotas({ agents, empty }: { agents: AgentRow[]; empty: string }) {
  const { colors, t, lang } = usePrefs();
  const meters = quotaMeters(agents, t);
  const credits = new Map<string, { label: string; balance: string }>();
  for (const agent of agents) {
    for (const [bucket, balance] of agent.credits ?? []) {
      const pool = bucket.replace(/\/credits$/, '');
      const label = pool === agent.agent ? agent.agent : `${agent.agent} · ${pool}`;
      credits.set(JSON.stringify([agent.agent, bucket]), { label, balance });
    }
  }
  return (
    <View style={{ gap: 10 }}>
      {credits.size > 0 ? (
        <View style={{ backgroundColor: colors.mine, padding: 10, borderRadius: 8, gap: 7 }}>
          <Text
            accessibilityRole="header"
            style={{ color: colors.text, fontSize: 14, fontWeight: '700' }}
          >
            {t('Credit balance')}
          </Text>
          {[...credits.entries()].map(([key, { label, balance }]) => (
            <View
              key={key}
              style={{
                flexDirection: 'row',
                flexWrap: 'wrap',
                alignItems: 'center',
                justifyContent: 'space-between',
                gap: 8,
              }}
            >
              <Text style={{ color: colors.text, fontSize: 14 }}>{label}</Text>
              <Text
                style={{
                  color: colors.text,
                  fontSize: 17,
                  fontWeight: '700',
                  fontVariant: ['tabular-nums'],
                }}
              >
                {balance === 'unlimited'
                  ? t('Unlimited')
                  : balance === 'unknown'
                    ? t('Unknown balance')
                    : t('{n} credits').replace('{n}', balance)}
              </Text>
            </View>
          ))}
        </View>
      ) : null}
      <Text
        accessibilityRole="header"
        style={{ color: colors.text, fontSize: 15, fontWeight: '700' }}
      >
        {t('Agent quota')}
      </Text>
      {meters.length ? (
        meters.map(([key, meter]) => {
          const percent = Math.round(meter.remaining * 100);
          const label = t(meter.estimated ? 'Estimated {n}% left' : '{n}% left').replace(
            '{n}',
            String(percent),
          );
          return (
            <View key={key} style={{ gap: 5 }}>
              <View
                style={{
                  flexDirection: 'row',
                  flexWrap: 'wrap',
                  justifyContent: 'space-between',
                  gap: 6,
                }}
              >
                <Text
                  style={{ color: colors.text, fontSize: 13, fontWeight: '600', flexShrink: 1 }}
                >
                  {meter.label}
                </Text>
                <Text style={{ color: colors.sub, fontSize: 13, fontVariant: ['tabular-nums'] }}>
                  {label}
                </Text>
              </View>
              <View
                accessibilityRole="progressbar"
                accessibilityLabel={meter.label}
                accessibilityValue={{ min: 0, now: percent, max: 100, text: label }}
                style={{
                  height: 5,
                  backgroundColor: colors.raised,
                  borderRadius: 3,
                  overflow: 'hidden',
                }}
              >
                <View
                  style={{ height: '100%', width: `${percent}%`, backgroundColor: colors.text }}
                />
              </View>
              {meter.reset !== null ? (
                <Text style={{ color: colors.sub, fontSize: 12 }}>
                  {t('Resets {time}').replace(
                    '{time}',
                    new Date(meter.reset).toLocaleString(lang, {
                      month: 'numeric',
                      day: 'numeric',
                      hour: 'numeric',
                      minute: '2-digit',
                    }),
                  )}
                </Text>
              ) : null}
            </View>
          );
        })
      ) : (
        <Text style={{ color: colors.sub, fontSize: 13 }}>{empty}</Text>
      )}
    </View>
  );
}
