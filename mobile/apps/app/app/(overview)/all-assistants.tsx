import React from 'react';
import { Pressable, Text, View } from 'react-native';
import { toMs } from '@orochi/core';
import type { Api } from '@orochi/protocol';
import { useAttention } from '../../src/state/attention';
import { useOpenAssistant } from '../../src/state/open';
import { usePrefs } from '../../src/state/prefs';
import { useWorkspaceReadings } from '../../src/state/workspaceReadings';
import { Icon } from '../../src/ui/Icon';
import { OverviewCards, WorkspaceHeading } from '../../src/ui/OverviewCards';
import { radius } from '../../src/ui/theme';

const readAssistants = async (remote: Api) => {
  const residents = await remote.residents();
  if (!Array.isArray(residents)) throw new Error('Invalid assistant response');
  return residents;
};

export default function AllAssistants() {
  const { colors, t, lang } = usePrefs();
  const { snapshots, refreshAll } = useAttention();
  const { readings, refresh } = useWorkspaceReadings(readAssistants);
  const open = useOpenAssistant();

  return (
    <OverviewCards
      title={t('Assistant')}
      snapshots={snapshots}
      refresh={async () => {
        await Promise.all([refreshAll(), refresh()]);
      }}
      renderItem={({ item: workspace }) => {
        const reading = readings[workspace.key];
        const residents = reading?.state === 'loaded' ? reading.data : [];
        return (
          <View style={{ gap: 10 }}>
            <WorkspaceHeading workspace={workspace} />
            {workspace.state === 'online' ? (
              !residents.length ? (
                <Text style={{ color: colors.sub, fontSize: 14 }}>
                  {reading?.state === 'loaded'
                    ? t('No assistants')
                    : reading?.state === 'unavailable'
                      ? t('Assistant status unavailable')
                      : t('Loading assistant status…')}
                </Text>
              ) : (
                residents.map((resident) => {
                  const next = toMs(resident.next_at);
                  const planned =
                    next !== null && !resident.paused
                      ? t('Next run {time}').replace(
                          '{time}',
                          new Date(next).toLocaleString(lang, {
                            month: 'short',
                            day: 'numeric',
                            hour: 'numeric',
                            minute: '2-digit',
                          }),
                        )
                      : undefined;
                  const status = [
                    t(resident.working ? 'Working' : resident.paused ? 'paused' : 'Idle'),
                    resident.asking > 0 ? t('Needs you') : undefined,
                  ]
                    .filter(Boolean)
                    .join(' · ');
                  return (
                    <Pressable
                      key={resident.thread_id}
                      accessibilityRole="button"
                      accessibilityLabel={[resident.name, status, planned]
                        .filter(Boolean)
                        .join(', ')}
                      onPress={() => void open(workspace.key, resident.thread_id)}
                      style={({ pressed }) => ({
                        backgroundColor: pressed ? colors.raised : colors.card,
                        borderColor: colors.line,
                        borderWidth: 1,
                        borderRadius: radius.card,
                        overflow: 'hidden',
                      })}
                    >
                      <View
                        style={{ flexDirection: 'row', alignItems: 'center', gap: 10, padding: 12 }}
                      >
                        <View style={{ backgroundColor: colors.mine, borderRadius: 9, padding: 9 }}>
                          <Icon name="sparkles-outline" size={20} color={colors.text} />
                        </View>
                        <View style={{ flex: 1, gap: 4 }}>
                          <Text style={{ color: colors.text, fontSize: 17, fontWeight: '700' }}>
                            {resident.name}
                          </Text>
                          <Text style={{ color: colors.sub, fontSize: 13 }}>{status}</Text>
                        </View>
                        <Icon name="chevron-forward" size={16} color={colors.sub} />
                      </View>
                      {planned ? (
                        <View
                          style={{
                            flexDirection: 'row',
                            alignItems: 'center',
                            gap: 8,
                            backgroundColor: colors.raised,
                            paddingHorizontal: 12,
                            paddingVertical: 10,
                          }}
                        >
                          <Icon name="calendar-outline" size={15} color={colors.sub} />
                          <Text style={{ color: colors.text, flex: 1, fontSize: 13 }}>
                            {planned}
                          </Text>
                        </View>
                      ) : null}
                    </Pressable>
                  );
                })
              )
            ) : null}
          </View>
        );
      }}
    />
  );
}
