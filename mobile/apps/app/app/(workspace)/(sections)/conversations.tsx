import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Pressable, RefreshControl, SectionList, Text, TextInput, View } from 'react-native';
import { useRouter } from 'expo-router';
import type { ProjectRow, SearchHit, ThreadRow } from '@orochi/protocol';
import {
  STATE_LABELS,
  STATUS_TONE,
  leadRoleOf,
  listSections,
  needsYou,
  since,
  whoOf,
} from '@orochi/core';
import { useData } from '../../../src/state/data';
import { usePrefs } from '../../../src/state/prefs';
import { useSession } from '../../../src/state/session';
import { WhoFace } from '../../../src/ui/people/WhoFace';
import { Icon } from '../../../src/ui/Icon';
import { DisclosureRow } from '../../../src/ui/Disclosure';
import { radius, toneInk } from '../../../src/ui/theme';
import { Empty } from '../../../src/ui/Screen';
import { ThreadMenu } from '../../../src/ui/ThreadMenu';
import { FAB, NewThreadButton } from '../../../src/ui/NewThread';

type Filter = 'all' | 'needs' | 'unread' | 'archived';

export default function Conversations() {
  const { projects, refresh, loaded, error } = useData();
  const { api } = useSession();
  const { colors, t } = usePrefs();
  const router = useRouter();
  const [query, setQuery] = useState('');
  const [filter, setFilter] = useState<Filter>('all');
  const [filtersOpen, setFiltersOpen] = useState(false);
  const [expandedProject, setExpandedProject] = useState<string | null>();
  const [refreshing, setRefreshing] = useState(false);
  const [searchResult, setSearchResult] = useState<{ query: string; hits: SearchHit[] } | null>(
    null,
  );
  const searchInput = useRef<React.ElementRef<typeof TextInput>>(null);
  const term = query.trim();
  const hits = filter === 'all' && searchResult?.query === term ? searchResult.hits : [];
  const [menu, setMenu] = useState<string | null>(null);
  const [archivedRows, setArchivedRows] = useState<ProjectRow[]>([]);

  const filtered = useMemo(() => {
    const keep = (th: ThreadRow) =>
      filter === 'needs' ? needsYou(th) : filter === 'unread' ? th.unread > 0 : true;
    return listSections(
      filter === 'archived' ? archivedRows : projects,
      term,
      filter === 'archived',
    )
      .map((s) => ({ ...s, threads: s.threads.filter(keep) }))
      .filter((s) => s.threads.length > 0 || (filter === 'all' && !term && s.kind === 'project'));
  }, [projects, archivedRows, term, filter]);
  const menuThread =
    [...projects, ...archivedRows].flatMap((p) => p.threads).find((th) => th.id === menu) ?? null;
  const loadArchived = useCallback(async () => {
    try {
      if (api) setArchivedRows(await api.sidebar(true));
    } catch {
      setArchivedRows([]);
    }
  }, [api]);
  useEffect(() => {
    if (filter === 'archived') loadArchived();
  }, [filter, loadArchived, projects]);

  const onRefresh = useCallback(async () => {
    setRefreshing(true);
    try {
      await refresh();
      if (filter === 'archived') await loadArchived();
    } finally {
      setRefreshing(false);
    }
  }, [filter, loadArchived, refresh]);

  useEffect(() => {
    if (!api || filter !== 'all' || term.length < 2) return;
    let active = true;
    const timer = setTimeout(async () => {
      try {
        const hits = await api.search(term);
        if (active) setSearchResult({ query: term, hits });
      } catch {
        if (active) setSearchResult(null);
      }
    }, 200);
    return () => {
      active = false;
      clearTimeout(timer);
    };
  }, [api, filter, term]);

  const open = (id: string) => router.push({ pathname: '/thread/[id]', params: { id } });
  const needsCount = projects.flatMap((p) => p.threads).filter(needsYou).length;
  const filterLabels: Record<Filter, string> = {
    all: t('All'),
    needs: t('Needs you'),
    unread: t('Unread'),
    archived: t('Archived'),
  };
  const showMatches = term !== '' || filter !== 'all';
  const projectOpen =
    expandedProject === undefined
      ? (
          filtered.find(
            (section) => section.kind === 'project' && section.threads.some(needsYou),
          ) ?? filtered.find((section) => section.kind === 'project' && section.threads.length > 0)
        )?.id
      : expandedProject;

  return (
    <View style={{ flex: 1, backgroundColor: colors.bg }}>
      <View style={{ paddingHorizontal: 18, paddingTop: 12, gap: 10 }}>
        <View
          style={{
            flexDirection: 'row',
            alignItems: 'center',
            backgroundColor: colors.card,
            borderRadius: 12,
            borderWidth: 1,
            borderColor: colors.line,
            paddingLeft: 12,
          }}
        >
          <Icon name="search" size={18} color={colors.sub} />
          <TextInput
            ref={searchInput}
            value={query}
            onChangeText={(value) => {
              if (value.trim() !== term) setSearchResult(null);
              setQuery(value);
            }}
            accessibilityLabel={t('Search conversations…')}
            returnKeyType="search"
            placeholder={t('Search conversations…')}
            placeholderTextColor={colors.sub}
            autoCorrect={false}
            autoCapitalize="none"
            style={{
              flex: 1,
              minWidth: 0,
              minHeight: 46,
              paddingHorizontal: 8,
              color: colors.text,
              fontSize: 16,
            }}
          />
          {query ? (
            <Pressable
              accessibilityRole="button"
              accessibilityLabel={t('Clear search')}
              onPress={() => {
                setQuery('');
                setSearchResult(null);
                searchInput.current?.focus();
              }}
              style={{ width: 44, height: 44, alignItems: 'center', justifyContent: 'center' }}
            >
              <Icon name="close-circle" size={20} color={colors.sub} />
            </Pressable>
          ) : null}
        </View>
        <View
          style={{
            flexDirection: 'row',
            alignItems: 'center',
            justifyContent: 'flex-end',
            gap: 8,
          }}
        >
          <Pressable
            accessibilityRole="button"
            aria-expanded={filtersOpen}
            accessibilityLabel={
              filter === 'all' ? t('Filters') : `${t('Filters')}, ${filterLabels[filter]}`
            }
            testID="conversation-filters"
            onPress={() => setFiltersOpen((value) => !value)}
            style={{
              minHeight: 44,
              paddingHorizontal: 8,
              gap: 6,
              flexDirection: 'row',
              alignItems: 'center',
              flexShrink: 1,
            }}
          >
            <Icon name="options-outline" size={19} color={colors.text} />
            <Text style={{ color: colors.text, fontSize: 14, fontWeight: '600', flexShrink: 1 }}>
              {filter === 'all' ? t('Filters') : filterLabels[filter]}
            </Text>
            <Icon name={filtersOpen ? 'chevron-up' : 'chevron-down'} size={13} color={colors.sub} />
          </Pressable>
        </View>
        {filtersOpen ? (
          <View
            style={{
              borderRadius: radius.card,
              borderWidth: 1,
              borderColor: colors.line,
              backgroundColor: colors.card,
            }}
          >
            {(Object.keys(filterLabels) as Filter[]).map((key) => (
              <Pressable
                key={key}
                onPress={() => {
                  setFilter(key);
                  setFiltersOpen(false);
                }}
                accessibilityRole="button"
                aria-pressed={filter === key}
                style={{
                  minHeight: 44,
                  flexDirection: 'row',
                  alignItems: 'center',
                  paddingHorizontal: 12,
                  gap: 8,
                }}
              >
                <Text style={{ flex: 1, color: colors.text, fontSize: 15 }}>
                  {filterLabels[key]}
                  {key === 'needs' && needsCount > 0 ? ` (${needsCount})` : ''}
                </Text>
                {key === filter ? <Icon name="checkmark" size={18} color={colors.text} /> : null}
              </Pressable>
            ))}
          </View>
        ) : null}
      </View>
      {error && !loaded ? <Empty text={`${t('Error')}: ${error}`} /> : null}
      {hits.length > 0 ? (
        <SectionList
          sections={[{ key: 'hits', title: t('Conversations'), data: hits }]}
          keyExtractor={(h) => h.thread_id}
          keyboardShouldPersistTaps="handled"
          contentContainerStyle={{ paddingTop: 8, paddingBottom: FAB + 36 }}
          renderItem={({ item }) => (
            <Pressable
              onPress={() => open(item.thread_id)}
              accessibilityRole="button"
              style={{
                marginHorizontal: 18,
                padding: 12,
                minHeight: 56,
                backgroundColor: colors.card,
                borderBottomWidth: 1,
                borderBottomColor: colors.line,
              }}
            >
              <Text style={{ color: colors.text, fontSize: 16, fontWeight: '600' }}>
                {item.title || t('New conversation')}
              </Text>
              <Text style={{ color: colors.sub, fontSize: 12 }}>{item.project}</Text>
              {item.snippet ? (
                <Text style={{ color: colors.sub, fontSize: 13 }} numberOfLines={2}>
                  {item.snippet}
                </Text>
              ) : null}
            </Pressable>
          )}
          renderSectionHeader={() => null}
        />
      ) : (
        <SectionList
          sections={filtered.map((s) => ({
            key: s.id,
            title: s.kind === 'pinned' ? t('Pinned') : s.title,
            expandable: !showMatches && s.kind === 'project' && s.threads.length > 0,
            count: s.threads.length,
            data: showMatches || s.kind === 'pinned' || s.id === projectOpen ? s.threads : [],
          }))}
          keyExtractor={(th) => th.id}
          refreshControl={
            <RefreshControl refreshing={refreshing} onRefresh={onRefresh} tintColor={colors.text} />
          }
          contentContainerStyle={{ paddingBottom: FAB + 36 }}
          keyboardShouldPersistTaps="handled"
          stickySectionHeadersEnabled={false}
          ListEmptyComponent={
            <Empty text={loaded ? t(term ? 'No matching conversations.' : 'Nothing here.') : ''} />
          }
          renderSectionHeader={({ section }) => (
            <View
              style={{
                marginHorizontal: 18,
                marginTop: 8,
                borderTopWidth: 1,
                borderTopColor: colors.line,
              }}
            >
              {section.expandable ? (
                <DisclosureRow
                  title={section.title}
                  summary={
                    projectOpen === section.key
                      ? undefined
                      : t(section.count === 1 ? '1 conversation' : '{n} conversations').replace(
                          '{n}',
                          String(section.count),
                        )
                  }
                  expanded={projectOpen === section.key}
                  testID={`project-${section.title}`}
                  onPress={() =>
                    setExpandedProject(projectOpen === section.key ? null : section.key)
                  }
                />
              ) : (
                <Text
                  accessibilityRole="header"
                  style={{ padding: 12, color: colors.sub, fontSize: 14, fontWeight: '600' }}
                >
                  {section.title}
                </Text>
              )}
            </View>
          )}
          renderItem={({ item, index }) => (
            <ThreadRowView
              th={item}
              separated={index > 0}
              onPress={() => open(item.id)}
              onLongPress={() => setMenu(item.id)}
            />
          )}
        />
      )}
      <ThreadMenu
        thread={menuThread}
        visible={menu !== null}
        onClose={() => setMenu(null)}
        onChanged={() => filter === 'archived' && loadArchived()}
      />
      <NewThreadButton />
    </View>
  );
}

function ThreadRowView({
  th,
  separated,
  onPress,
  onLongPress,
}: {
  th: ThreadRow;
  separated: boolean;
  onPress: () => void;
  onLongPress: () => void;
}) {
  const { colors, t } = usePrefs();
  const title = th.title || t('New conversation');
  const status = t(STATE_LABELS[th.status]);
  const attention = th.asking > 0 || (th.mentions ?? 0) > 0;
  const badge =
    th.asking > 0 ? '!' : (th.mentions ?? 0) > 0 ? '@' : th.unread > 99 ? '99+' : th.unread;
  const faces = th.agents
    .slice(0, 3)
    .map((a) => whoOf({ name: a.role, ...a, role: a.role || 'lead' }, leadRoleOf(th)));

  return (
    <View
      style={{
        marginHorizontal: 18,
        flexDirection: 'row',
        alignItems: 'center',
        borderTopWidth: separated ? 1 : 0,
        borderTopColor: colors.line,
      }}
    >
      <Pressable
        onPress={onPress}
        onLongPress={onLongPress}
        delayLongPress={350}
        accessibilityRole="button"
        accessibilityLabel={`${title}, ${status}${attention && th.status !== 'needs_you' ? `, ${t('Needs you')}` : ''}`}
        style={({ pressed }) => ({
          flex: 1,
          minHeight: 66,
          justifyContent: 'center',
          gap: 5,
          paddingLeft: 12,
          paddingVertical: 10,
          borderRadius: radius.card,
          backgroundColor: pressed ? colors.raised : undefined,
        })}
      >
        <View style={{ flexDirection: 'row', alignItems: 'center', gap: 6 }}>
          {th.pinned ? <Icon name="pin" size={12} color={colors.sub} /> : null}
          <Text
            style={{ color: colors.text, fontSize: 16, fontWeight: '600', flex: 1 }}
            numberOfLines={2}
          >
            {title}
          </Text>
          <Text style={{ color: colors.sub, fontSize: 12 }} numberOfLines={1}>
            {since(th.running_since || th.updated_at)}
          </Text>
        </View>
        <View style={{ flexDirection: 'row', alignItems: 'center', gap: 6 }}>
          {faces.length > 0 ? (
            <View style={{ flexDirection: 'row' }}>
              {faces.map((a, i) => (
                <View key={i} style={{ marginLeft: i ? -5 : 0 }}>
                  <WhoFace who={a} size={20} badge={false} />
                </View>
              ))}
            </View>
          ) : null}
          <Text
            style={{
              color: toneInk(colors, STATUS_TONE[th.status]),
              fontSize: 13,
              fontWeight: attention ? '700' : '500',
              flexShrink: 1,
            }}
            numberOfLines={1}
          >
            {status}
          </Text>
          {th.doing ? (
            <Text style={{ color: colors.sub, fontSize: 13, flex: 1 }} numberOfLines={1}>
              {th.doing.split('\n')[0]}
            </Text>
          ) : (
            <View style={{ flex: 1 }} />
          )}
          {badge ? (
            <View
              style={{
                minWidth: 20,
                minHeight: 20,
                borderRadius: 10,
                paddingHorizontal: 5,
                alignItems: 'center',
                justifyContent: 'center',
                backgroundColor: attention ? colors.attention : colors.mine,
              }}
            >
              <Text
                style={{
                  color: attention ? colors.onPrimary : colors.text,
                  fontSize: 11,
                  fontWeight: '700',
                }}
              >
                {badge}
              </Text>
            </View>
          ) : null}
        </View>
      </Pressable>
      <Pressable
        onPress={onLongPress}
        accessibilityRole="button"
        accessibilityLabel={`${t('More')}: ${title}`}
        style={({ pressed }) => ({
          width: 44,
          height: 44,
          alignItems: 'center',
          justifyContent: 'center',
          borderRadius: radius.tile,
          backgroundColor: pressed ? colors.raised : undefined,
        })}
      >
        <Icon name="ellipsis-horizontal" size={20} color={colors.sub} />
      </Pressable>
    </View>
  );
}
