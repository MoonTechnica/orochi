import React, { useState } from 'react';
import { Text, View } from 'react-native';
import { activityTree, type MachineSnapshot } from '@orochi/core';
import type { Api } from '@orochi/protocol';
import { useAttention } from '../../src/state/attention';
import { useOpenThread } from '../../src/state/open';
import { usePrefs } from '../../src/state/prefs';
import { useWorkspaceReadings } from '../../src/state/workspaceReadings';
import { AgentQuotas } from '../../src/ui/AgentQuotas';
import { Disclosure, DisclosureRow } from '../../src/ui/Disclosure';
import { OverviewCards, WorkspaceHeading } from '../../src/ui/OverviewCards';
import { radius } from '../../src/ui/theme';

const readAgents = async (remote: Api) => {
  const agents = await remote.agents();
  if (!Array.isArray(agents)) throw new Error('Invalid agent response');
  return agents;
};

export default function AllActivity() {
  const { t } = usePrefs();
  const { snapshots, refreshAll } = useAttention();
  const { readings, refresh } = useWorkspaceReadings(readAgents);

  return (
    <OverviewCards
      title={t('Activity')}
      snapshots={snapshots}
      refresh={async () => {
        await Promise.all([refreshAll(), refresh()]);
      }}
      renderItem={({ item: workspace }) => {
        const reading = readings[workspace.key];
        return (
          <ActivityCard workspace={workspace}>
            <AgentQuotas
              agents={reading?.state === 'loaded' ? reading.data : []}
              empty={
                reading?.state === 'loaded'
                  ? t('No quota data')
                  : reading?.state === 'unavailable'
                    ? t('Agent quota unavailable')
                    : t('Loading agent quota…')
              }
            />
          </ActivityCard>
        );
      }}
    />
  );
}

function ActivityCard({
  workspace,
  children,
}: {
  workspace: MachineSnapshot;
  children: React.ReactNode;
}) {
  const { colors, t } = usePrefs();
  const open = useOpenThread();
  const [expanded, setExpanded] = useState(false);
  const asking = workspace.seats.filter((seat) => seat.state === 'asking').length;
  const nodes = expanded ? activityTree(workspace.seats, workspace.tasks, workspace.projects) : [];
  const detail = (n: number) => t(n === 1 ? '1 agent' : '{n} agents').replace('{n}', String(n));

  return (
    <View
      style={{
        backgroundColor: colors.card,
        borderRadius: radius.card,
        borderWidth: 1,
        borderColor: colors.line,
        overflow: 'hidden',
      }}
    >
      <View style={{ padding: 12 }}>
        <WorkspaceHeading workspace={workspace} />
      </View>
      {workspace.state === 'online' ? (
        <>
          <View
            style={{
              flexDirection: 'row',
              marginHorizontal: 12,
              backgroundColor: colors.raised,
              borderRadius: 9,
            }}
          >
            {[
              { label: t('Working'), count: workspace.seats.length - asking },
              { label: t('Needs you'), count: asking },
              { label: t('Sub-agents'), count: workspace.tasks.length },
            ].map(({ label, count }, index) => (
              <View
                key={label}
                style={{
                  flex: index === 2 ? 1.6 : 1,
                  alignItems: 'center',
                  gap: 3,
                  paddingVertical: 9,
                  paddingHorizontal: 4,
                  borderLeftWidth: index === 0 ? 0 : 1,
                  borderLeftColor: colors.line,
                }}
              >
                <Text
                  style={{
                    color: colors.text,
                    fontSize: 24,
                    fontWeight: '800',
                    fontVariant: ['tabular-nums'],
                  }}
                >
                  {count}
                </Text>
                <Text style={{ color: colors.sub, fontSize: 12, textAlign: 'center' }}>
                  {label}
                </Text>
              </View>
            ))}
          </View>
          <View style={{ padding: 12 }}>{children}</View>
          {workspace.seats.length > 0 ? (
            <Disclosure
              title={t('Live conversations')}
              expanded={expanded}
              onPress={() => setExpanded((value) => !value)}
            >
              {nodes.map((node) => (
                <DisclosureRow
                  key={node.thread_id}
                  title={node.title}
                  summary={[node.project, detail(node.seats.length)].filter(Boolean).join(' · ')}
                  onPress={() => void open(workspace.key, node.thread_id)}
                />
              ))}
            </Disclosure>
          ) : null}
        </>
      ) : null}
    </View>
  );
}
