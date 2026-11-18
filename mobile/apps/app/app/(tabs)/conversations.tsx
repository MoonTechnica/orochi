import React, { useCallback, useMemo, useState } from 'react';
import { Pressable, RefreshControl, SectionList, Text, TextInput, View } from 'react-native';
import { useRouter } from 'expo-router';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import type { ProjectRow, SearchHit, ThreadRow } from '@orochi/protocol';
import {
  STATE_LABELS,
  STATUS_MARK,
  STATUS_TONE,
  leadRoleOf,
  listSections,
  needsYou,
  since,
  t as tr,
  whoOf,
} from '@orochi/core';
import { useData } from '../../src/state/data';
import { usePrefs } from '../../src/state/prefs';
import { toneInk } from '../../src/ui/theme';
import { useSession } from '../../src/state/session';
import { WhoFace } from '../../src/ui/people/WhoFace';
import { Icon } from '../../src/ui/Icon';
import { MachineSwitcher } from '../../src/ui/Machines';
import { Empty } from '../../src/ui/Screen';
import { ThreadMenu } from '../../src/ui/ThreadMenu';

type Filter = 'all' | 'needs' | 'unread' | 'archived';

export default function Conversations() {
  const { projects, refresh, connection, loaded, error } = useData();
  const { api } = useSession();
  const { colors, t, lang } = usePrefs();
  const router = useRouter();
  const insets = useSafeAreaInsets();
  const [query, setQuery] = useState('');
  const [filter, setFilter] = useState<Filter>('all');
  const [refreshing, setRefreshing] = useState(false);
  const [hits, setHits] = useState<SearchHit[]>([]);
  const [menu, setMenu] = useState<string | null>(null);
  const [archivedRows, setArchivedRows] = useState<ProjectRow[]>([]);

  const filtered = useMemo(() => {
    const keep = (th: ThreadRow) =>
      filter === 'needs' ? needsYou(th) : filter === 'unread' ? th.unread > 0 : true;
    return listSections(
      filter === 'archived' ? archivedRows : projects,
      query,
      filter === 'archived',
    )
      .map((s) => ({ ...s, threads: s.threads.filter(keep) }))
      .filter((s) => s.threads.length > 0 || (filter === 'all' && !query && s.kind === 'project'));
  }, [projects, archivedRows, query, filter]);
  const menuThread =
    [...projects, ...archivedRows].flatMap((p) => p.threads).find((th) => th.id === menu) ?? null;
  const loadArchived = useCallback(async () => {
    try {
      if (api) setArchivedRows(await api.sidebar(true));
    } catch {
      setArchivedRows([]);
    }
  }, [api]);
  React.useEffect(() => {
    if (filter === 'archived') loadArchived();
  }, [filter, loadArchived, projects]);

  const onRefresh = useCallback(async () => {
    setRefreshing(true);
    await refresh();
    setRefreshing(false);
  }, [refresh]);

  const search = async (text: string) => {
    setQuery(text);
    if (!api || text.trim().length < 2) return setHits([]);
    try {
      const found = await api.search(text);
      setHits(found);
    } catch {
      setHits([]);
    }
  };

  const create = async (root: string) => {
    if (!api) return;
    const id = await api.newThread(root);
    router.push({ pathname: '/thread/[id]', params: { id } });
  };

  const open = (id: string) => router.push({ pathname: '/thread/[id]', params: { id } });
  const needsCount = projects.flatMap((p) => p.threads).filter(needsYou).length;

  return (
    <View style={{ flex: 1, backgroundColor: colors.bg, paddingTop: insets.top }}>
      <View style={{ paddingHorizontal: 16, paddingTop: 10, gap: 10 }}>
        <View style={{ flexDirection: 'row', alignItems: 'center', gap: 8 }}>
          <MachineSwitcher />
        </View>
        <View
          style={{
            flexDirection: 'row',
            alignItems: 'center',
            backgroundColor: colors.card,
            borderRadius: 12,
            borderWidth: 1,
            borderColor: colors.line,
            paddingHorizontal: 10,
          }}
        >
          <Icon name="search" size={18} color={colors.sub} />
          <TextInput
            value={query}
            onChangeText={search}
            placeholder={t('Search conversations…')}
            placeholderTextColor={colors.sub}
            autoCorrect={false}
            autoCapitalize="none"
            style={{
              flex: 1,
              minHeight: 44,
              paddingHorizontal: 8,
              color: colors.text,
              fontSize: 16,
            }}
          />
        </View>
        <View style={{ flexDirection: 'row', gap: 8, flexWrap: 'wrap' }}>
          {(
            [
              ['all', t('Conversations')],
              ['needs', `${t('Needs you')}${needsCount ? ` (${needsCount})` : ''}`],
              ['unread', t('Unread')],
              ['archived', t('Archived')],
            ] as [Filter, string][]
          ).map(([key, label]) => (
            <Pressable
              key={key}
              onPress={() => setFilter(key)}
              accessibilityRole="button"
              accessibilityState={{ selected: filter === key }}
              style={{
                minHeight: 36,
                paddingHorizontal: 14,
                borderRadius: 8,
                justifyContent: 'center',
                backgroundColor: filter === key ? colors.text : colors.card,
                borderWidth: 1,
                borderColor: filter === key ? colors.text : colors.line,
              }}
            >
              <Text
                style={{
                  color: filter === key ? colors.bg : colors.text,
                  fontSize: 13,
                  fontWeight: '600',
                }}
              >
                {label}
              </Text>
            </Pressable>
          ))}
        </View>
      </View>
      {error && !loaded ? <Empty text={`${t('Error')}: ${error}`} /> : null}
      {hits.length > 0 ? (
        <SectionList
          sections={[{ key: 'hits', title: t('Conversations'), data: hits }]}
          keyExtractor={(h) => h.thread_id}
          renderItem={({ item }) => (
            <Pressable
              onPress={() => open(item.thread_id)}
              style={{ paddingHorizontal: 16, paddingVertical: 12, minHeight: 56 }}
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
            root: s.project?.root,
            data: s.threads,
          }))}
          keyExtractor={(th) => th.id}
          refreshControl={
            <RefreshControl refreshing={refreshing} onRefresh={onRefresh} tintColor={colors.text} />
          }
          contentContainerStyle={{ paddingBottom: 24 }}
          stickySectionHeadersEnabled={false}
          ListEmptyComponent={<Empty text={loaded ? t('Nothing here.') : ''} />}
          renderSectionHeader={({ section }) => (
            <View
              style={{
                flexDirection: 'row',
                alignItems: 'center',
                paddingHorizontal: 16,
                paddingTop: 18,
                paddingBottom: 6,
              }}
            >
              <Text
                style={{
                  flex: 1,
                  color: colors.sub,
                  fontSize: 13,
                  fontWeight: '700',
                  textTransform: 'uppercase',
                }}
                numberOfLines={1}
              >
                {section.title}
              </Text>
              {section.root ? (
                <Pressable
                  onPress={() => create(section.root!)}
                  accessibilityRole="button"
                  accessibilityLabel={t('New thread')}
                  style={{
                    width: 44,
                    height: 36,
                    alignItems: 'flex-end',
                    justifyContent: 'center',
                  }}
                >
                  <Icon name="add-circle-outline" size={24} color={colors.text} />
                </Pressable>
              ) : null}
            </View>
          )}
          renderItem={({ item }) => (
            <ThreadRowView
              th={item}
              onPress={() => open(item.id)}
              onLongPress={() => setMenu(item.id)}
              lang={lang}
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
    </View>
  );
}

function ThreadRowView({
  th,
  onPress,
  onLongPress,
  lang,
}: {
  th: ThreadRow;
  onPress: () => void;
  onLongPress: () => void;
  lang: 'en' | 'ja';
}) {
  const { colors, t } = usePrefs();
  const tone = STATUS_TONE[th.status];
  const color = toneInk(colors, tone);
  const faces = th.agents
    .slice(0, 3)
    .map((a) => whoOf({ name: a.role, ...a, role: a.role || 'lead' }, leadRoleOf(th)));
  return (
    <View
      style={{
        flexDirection: 'row',
        alignItems: 'stretch',
        paddingRight: 8,
        minHeight: 64,
        backgroundColor: colors.card,
        borderBottomWidth: 1,
        borderBottomColor: colors.line,
      }}
    >
      <Pressable
        onPress={onPress}
        onLongPress={onLongPress}
        delayLongPress={350}
        accessibilityRole="button"
        accessibilityLabel={`${th.title}, ${tr(STATE_LABELS[th.status], lang)}`}
        style={({ pressed }) => ({
          flex: 1,
          flexDirection: 'row',
          gap: 12,
          paddingLeft: 16,
          paddingVertical: 12,
          backgroundColor: pressed ? colors.code : undefined,
        })}
      >
        <View style={{ width: 24, alignItems: 'center', paddingTop: 2 }}>
          <Text style={{ color, fontSize: 20, fontWeight: '800' }}>{STATUS_MARK[th.status]}</Text>
        </View>
        <View style={{ flex: 1, gap: 3 }}>
          <View style={{ flexDirection: 'row', alignItems: 'center', gap: 6 }}>
            {th.pinned ? <Icon name="pin" size={12} color={colors.sub} /> : null}
            <Text
              style={{
                color: colors.text,
                fontSize: 16,
                fontWeight: th.unread > 0 ? '700' : '500',
                flex: 1,
              }}
              numberOfLines={1}
            >
              {th.title || t('New conversation')}
            </Text>
            <Text style={{ color: colors.sub, fontSize: 12 }}>
              {since(th.running_since || th.updated_at)}
            </Text>
          </View>
          <View style={{ flexDirection: 'row', alignItems: 'center', gap: 6 }}>
            <View style={{ flexDirection: 'row' }}>
              {faces.map((a, i) => (
                <View key={i} style={{ marginLeft: i ? -6 : 0 }}>
                  <WhoFace who={a} size={22} badge={false} />
                </View>
              ))}
            </View>
            <Text
              style={{
                color: th.status === 'idle' ? colors.sub : color,
                fontSize: 12,
                fontWeight: '600',
              }}
              numberOfLines={1}
            >
              {t(STATE_LABELS[th.status])}
            </Text>
            {th.doing ? (
              <Text style={{ color: colors.sub, fontSize: 12, flex: 1 }} numberOfLines={1}>
                {th.doing.split('\n')[0]}
              </Text>
            ) : null}
          </View>
        </View>
      </Pressable>
      <Pressable
        onPress={onLongPress}
        accessibilityRole="button"
        accessibilityLabel={`${t('More')}: ${th.title}`}
        hitSlop={6}
        style={{
          width: 32,
          alignSelf: 'center',
          alignItems: 'center',
          height: 44,
          justifyContent: 'center',
        }}
      >
        <Icon name="ellipsis-horizontal" size={20} color={colors.sub} />
      </Pressable>
      <View style={{ width: 36, alignSelf: 'center', alignItems: 'center' }}>
        {th.unread > 0 || th.asking > 0 || (th.mentions ?? 0) > 0 ? (
          <View
            style={{
              alignSelf: 'center',
              minWidth: 22,
              height: 22,
              borderRadius: 11,
              paddingHorizontal: 6,
              backgroundColor:
                th.asking > 0 || (th.mentions ?? 0) > 0 ? colors.attention : colors.text,
              alignItems: 'center',
              justifyContent: 'center',
            }}
          >
            <Text
              style={{
                color: th.asking > 0 || (th.mentions ?? 0) > 0 ? colors.onPrimary : colors.bg,
                fontSize: 12,
                fontWeight: '700',
              }}
            >
              {th.asking > 0 ? '!' : (th.mentions ?? 0) > 0 ? '@' : th.unread}
            </Text>
          </View>
        ) : null}
      </View>
    </View>
  );
}
