import React, { useState } from 'react';
import { Pressable, SectionList, Text, View } from 'react-native';
import { usePrefs } from '../state/prefs';
import type { MachineState } from '../state/attention';
import { DisclosureRow } from './Disclosure';
import { WorkspaceHeading } from './OverviewCards';
import { useOverviewRefresh } from './useOverviewRefresh';
import { radius } from './theme';
import { Icon } from './Icon';
import { Empty, Header } from './Screen';

interface Item {
  key: string;
  title: string;
  summary: string;
  status: string;
  needsReply: boolean;
  onPress(): void;
}

export interface WorkspaceSection {
  key: string;
  title: string;
  state: MachineState;
  data: Item[];
  empty: string;
  summary?: string;
}

const PREVIEW = 3;

export function ConversationInbox({
  title,
  sections,
  refresh,
  filters,
}: {
  title: string;
  sections: WorkspaceSection[];
  refresh(): Promise<void>;
  filters: { label: string; selected: boolean; onPress(): void }[];
}) {
  const { colors, t } = usePrefs();
  const refreshControl = useOverviewRefresh(refresh);
  const [expanded, setExpanded] = useState<Record<string, boolean>>({});
  return (
    <View style={{ flex: 1, backgroundColor: colors.bg }}>
      <Header title={title} subtitle={t('All workspaces')} back={false} />
      <SectionList
        sections={sections.map((section) => ({
          ...section,
          total: section.data.length,
          data: expanded[section.key] ? section.data : section.data.slice(0, PREVIEW),
        }))}
        keyExtractor={(item) => item.key}
        contentContainerStyle={{ padding: 16 }}
        stickySectionHeadersEnabled={false}
        refreshControl={refreshControl}
        ListHeaderComponent={
          <View style={{ flexDirection: 'row', gap: 6, flexWrap: 'wrap', marginBottom: 18 }}>
            {filters.map((filter) => (
              <Pressable
                key={filter.label}
                accessibilityRole="button"
                aria-pressed={filter.selected}
                onPress={filter.onPress}
                style={{
                  minHeight: 44,
                  paddingHorizontal: 12,
                  justifyContent: 'center',
                  borderRadius: radius.chip,
                  backgroundColor: filter.selected ? colors.primary : colors.raised,
                }}
              >
                <Text
                  style={{
                    color: filter.selected ? colors.onPrimary : colors.text,
                    fontSize: 13,
                    fontWeight: '600',
                  }}
                >
                  {filter.label}
                </Text>
              </Pressable>
            ))}
          </View>
        }
        ListEmptyComponent={<Empty text={t('Pair a desktop to begin')} />}
        renderSectionHeader={({ section }) => (
          <View style={{ gap: 3, paddingBottom: 8 }}>
            <WorkspaceHeading
              workspace={{ key: section.key, name: section.title, state: section.state }}
            />
            {section.summary ? (
              <Text style={{ color: colors.sub, fontSize: 13 }}>{section.summary}</Text>
            ) : null}
          </View>
        )}
        renderItem={({ item }) => (
          <Pressable
            accessibilityRole="button"
            accessibilityLabel={[item.title, item.status, item.summary].filter(Boolean).join(', ')}
            onPress={item.onPress}
            style={({ pressed }) => ({
              borderTopWidth: 1,
              borderTopColor: colors.line,
              backgroundColor: pressed ? colors.raised : 'transparent',
              paddingVertical: 10,
              minHeight: 52,
              flexDirection: 'row',
              alignItems: 'center',
              gap: 8,
            })}
          >
            <View style={{ flex: 1, gap: 4 }}>
              <Text
                style={{ color: colors.text, fontSize: 15, fontWeight: '600' }}
                numberOfLines={2}
              >
                {item.title}
              </Text>
              <Text style={{ color: colors.sub, fontSize: 12 }} numberOfLines={2}>
                {item.summary}
              </Text>
            </View>
            <View
              style={{
                paddingHorizontal: 7,
                paddingVertical: 4,
                borderRadius: radius.chip,
                backgroundColor: item.needsReply ? colors.primary : colors.raised,
                maxWidth: '40%',
              }}
            >
              <Text
                style={{
                  color: item.needsReply ? colors.onPrimary : colors.sub,
                  fontSize: 12,
                  fontWeight: '600',
                }}
              >
                {item.status}
              </Text>
            </View>
            <Icon name="chevron-forward" size={14} color={colors.sub} />
          </Pressable>
        )}
        renderSectionFooter={({ section }) => (
          <View style={{ paddingBottom: 20 }}>
            {section.state === 'online' && section.total === 0 ? (
              <Text style={{ color: colors.sub, paddingHorizontal: 12, paddingVertical: 8 }}>
                {section.empty}
              </Text>
            ) : null}
            {section.total > PREVIEW ? (
              <DisclosureRow
                title={t(expanded[section.key] ? 'Show fewer' : 'Show all')}
                expanded={!!expanded[section.key]}
                onPress={() =>
                  setExpanded((current) => ({ ...current, [section.key]: !current[section.key] }))
                }
              />
            ) : null}
          </View>
        )}
      />
    </View>
  );
}
