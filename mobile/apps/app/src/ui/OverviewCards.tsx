import React from 'react';
import { FlatList, Text, View, type ListRenderItem } from 'react-native';
import { displayName, type MachineSnapshot } from '@orochi/core';
import { usePrefs } from '../state/prefs';
import { useSession } from '../state/session';
import { ConnectionBadge } from './Machines';
import { Empty, Header } from './Screen';
import { useOverviewRefresh } from './useOverviewRefresh';

export function WorkspaceName({
  workspace,
  heading,
}: {
  workspace: Pick<MachineSnapshot, 'key' | 'name'>;
  heading?: boolean;
}) {
  const { colors } = usePrefs();
  const { machines } = useSession();
  const machine = machines.find((item) => item.key === workspace.key);
  const name = machine ? displayName(machine) : workspace.name;
  const address =
    machine && name !== workspace.name ? `${machine.host}:${machine.port}` : undefined;
  return (
    <View style={{ flexShrink: 1, gap: 2 }}>
      <Text
        accessibilityRole={heading ? 'header' : undefined}
        numberOfLines={2}
        style={{ color: colors.text, fontSize: 16, fontWeight: '700' }}
      >
        {name}
      </Text>
      {address ? (
        <Text numberOfLines={1} style={{ color: colors.sub, fontSize: 12 }}>
          {address}
        </Text>
      ) : null}
    </View>
  );
}

export function WorkspaceHeading({
  workspace,
}: {
  workspace: Pick<MachineSnapshot, 'key' | 'name' | 'state'>;
}) {
  return (
    <View
      style={{
        flexDirection: 'row',
        alignItems: 'center',
        justifyContent: 'space-between',
        gap: 8,
      }}
    >
      <WorkspaceName workspace={workspace} heading />
      <ConnectionBadge state={workspace.state} />
    </View>
  );
}

/** Scroll and refresh workspaces. Each overview supplies its own card layout. */
export function OverviewCards({
  title,
  snapshots,
  refresh,
  renderItem,
}: {
  title: string;
  snapshots: MachineSnapshot[];
  refresh(): Promise<void>;
  renderItem: ListRenderItem<MachineSnapshot>;
}) {
  const { colors, t } = usePrefs();
  const refreshControl = useOverviewRefresh(refresh);
  return (
    <View style={{ flex: 1, backgroundColor: colors.bg }}>
      <Header title={title} subtitle={t('All workspaces')} back={false} />
      <FlatList
        data={snapshots}
        keyExtractor={(workspace) => workspace.key}
        renderItem={renderItem}
        contentContainerStyle={{ padding: 16, gap: 14 }}
        refreshControl={refreshControl}
        ListEmptyComponent={<Empty text={t('Pair a desktop to begin')} />}
      />
    </View>
  );
}
