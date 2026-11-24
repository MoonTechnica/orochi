import React, { useCallback, useEffect, useState } from 'react';
import { FlatList, Pressable, ScrollView, Text, TextInput, View } from 'react-native';
import { useLocalSearchParams, useRouter } from 'expo-router';
import { crumbs, parentDir, sizeLabel, sortTree } from '@orochi/core';
import type { DiffScope, FileRow, TreeEntry, TreeHit } from '@orochi/protocol';
import { usePrefs } from '../../../../src/state/prefs';
import { useSession } from '../../../../src/state/session';
import { useData } from '../../../../src/state/data';
import { Icon } from '../../../../src/ui/Icon';
import { Empty, Header } from '../../../../src/ui/Screen';

type Mode = 'browse' | 'changed';
const SCOPES: DiffScope[] = ['unstaged', 'staged', 'branch'];
const STATE_MARK: Record<string, string> = {
  modified: 'M',
  added: 'A',
  deleted: 'D',
  renamed: 'R',
  untracked: 'U',
  conflict: '!',
  ignored: '·',
};

export default function Files() {
  const { id } = useLocalSearchParams<{ id: string }>();
  const { api } = useSession();
  const { colors, t } = usePrefs();
  const { onChanged } = useData();
  const router = useRouter();
  const [mode, setMode] = useState<Mode>('browse');
  const [dir, setDir] = useState('');
  const [entries, setEntries] = useState<TreeEntry[]>([]);
  const [scope, setScope] = useState<DiffScope>('unstaged');
  const [changed, setChanged] = useState<FileRow[]>([]);
  const [query, setQuery] = useState('');
  const [by, setBy] = useState<'name' | 'content'>('name');
  const [hits, setHits] = useState<TreeHit[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    if (!api) return;
    try {
      if (mode === 'browse') setEntries(sortTree(await api.treeList(id, dir)));
      else setChanged(await api.treeFiles(id, scope));
      setError(null);
    } catch (e: any) {
      setError(String(e?.message ?? e));
    }
  }, [api, id, mode, dir, scope]);
  useEffect(() => {
    load();
  }, [load]);
  useEffect(
    () => onChanged((ids) => (ids.length === 0 || ids.includes(id)) && load()),
    [onChanged, id, load],
  );

  useEffect(() => {
    if (!api || query.trim().length < 2) return setHits(null);
    const timer = setTimeout(async () => {
      try {
        setHits(await api.treeFind(id, query.trim(), by));
      } catch {
        setHits([]);
      }
    }, 300);
    return () => clearTimeout(timer);
  }, [api, id, query, by]);

  const open = (path: string, extra: Record<string, string> = {}, line?: number) =>
    router.push({
      pathname: '/viewer/file',
      params: { thread: id, path, ...(line ? { line: String(line) } : {}), ...extra },
    });

  const chip = (label: string, on: boolean, press: () => void) => (
    <Pressable
      key={label}
      onPress={press}
      accessibilityRole="button"
      accessibilityState={{ selected: on }}
      style={{
        minHeight: 34,
        paddingHorizontal: 12,
        borderRadius: 8,
        justifyContent: 'center',
        backgroundColor: on ? colors.text : colors.card,
        borderWidth: 1,
        borderColor: on ? colors.text : colors.line,
      }}
    >
      <Text style={{ color: on ? colors.bg : colors.text, fontSize: 13, fontWeight: '600' }}>
        {label}
      </Text>
    </Pressable>
  );

  return (
    <View style={{ flex: 1, backgroundColor: colors.bg }}>
      <Header title={t('Files')} />
      <View style={{ flexDirection: 'row', gap: 8, padding: 10 }}>
        {chip(t('Browse'), mode === 'browse', () => setMode('browse'))}
        {chip(t('Changed'), mode === 'changed', () => setMode('changed'))}
      </View>
      {mode === 'browse' ? (
        <View style={{ paddingHorizontal: 10, gap: 8 }}>
          <TextInput
            value={query}
            onChangeText={setQuery}
            placeholder={t('Find files…')}
            placeholderTextColor={colors.sub}
            autoCapitalize="none"
            autoCorrect={false}
            accessibilityLabel={t('Find files…')}
            style={{
              minHeight: 44,
              borderRadius: 10,
              borderWidth: 1,
              borderColor: colors.line,
              paddingHorizontal: 12,
              color: colors.text,
              backgroundColor: colors.card,
              fontSize: 16,
            }}
          />
          {query.trim().length >= 2 ? (
            <View style={{ flexDirection: 'row', gap: 8 }}>
              {chip(t('Names'), by === 'name', () => setBy('name'))}
              {chip(t('Contents'), by === 'content', () => setBy('content'))}
            </View>
          ) : (
            <ScrollView
              horizontal
              showsHorizontalScrollIndicator={false}
              contentContainerStyle={{ gap: 6, alignItems: 'center' }}
            >
              <Pressable
                onPress={() => setDir('')}
                style={{ minHeight: 36, justifyContent: 'center' }}
              >
                <Text style={{ color: colors.text, fontSize: 14 }}>/</Text>
              </Pressable>
              {crumbs(dir).map((c) => (
                <Pressable
                  key={c.path}
                  onPress={() => setDir(c.path)}
                  style={{ minHeight: 36, justifyContent: 'center' }}
                >
                  <Text style={{ color: colors.text, fontSize: 14 }}>{c.name} /</Text>
                </Pressable>
              ))}
            </ScrollView>
          )}
        </View>
      ) : (
        <View style={{ flexDirection: 'row', gap: 8, paddingHorizontal: 10, paddingBottom: 8 }}>
          {SCOPES.map((s) => chip(t(s), scope === s, () => setScope(s)))}
        </View>
      )}
      {error ? <Empty text={error} /> : null}
      {mode === 'browse' && hits ? (
        <FlatList
          data={hits}
          keyExtractor={(h, i) => `${h.path}:${h.line}:${i}`}
          ListEmptyComponent={<Empty text={t('Nothing here.')} />}
          renderItem={({ item }) => (
            <Pressable
              onPress={() => open(item.path, {}, item.line ?? undefined)}
              style={{
                padding: 12,
                borderBottomWidth: 1,
                borderBottomColor: colors.line,
                backgroundColor: colors.card,
              }}
            >
              <Text style={{ color: colors.text, fontSize: 15 }} numberOfLines={1}>
                {item.path}
                {item.line ? `:${item.line}` : ''}
              </Text>
              {item.text ? (
                <Text style={{ color: colors.sub, fontSize: 12 }} numberOfLines={2}>
                  {item.text}
                </Text>
              ) : null}
            </Pressable>
          )}
        />
      ) : mode === 'browse' ? (
        <FlatList
          data={
            dir
              ? [{ name: '..', path: parentDir(dir), kind: 'dir' } as TreeEntry, ...entries]
              : entries
          }
          keyExtractor={(e) => `${e.path}-${e.name}`}
          ListEmptyComponent={<Empty text={t('Nothing here.')} />}
          renderItem={({ item }) => {
            const state = item.status ?? item.within;
            return (
              <Pressable
                disabled={item.missing}
                onPress={() => (item.kind === 'dir' ? setDir(item.path) : open(item.path))}
                accessibilityRole="button"
                accessibilityLabel={item.name}
                style={{
                  minHeight: 48,
                  flexDirection: 'row',
                  alignItems: 'center',
                  gap: 10,
                  paddingHorizontal: 16,
                  borderBottomWidth: 1,
                  borderBottomColor: colors.line,
                  backgroundColor: colors.card,
                  opacity: item.missing ? 0.5 : 1,
                }}
              >
                <Icon
                  name={item.kind === 'dir' ? 'folder' : 'document-text-outline'}
                  size={20}
                  color={colors.sub}
                />
                <Text style={{ color: colors.text, fontSize: 16, flex: 1 }} numberOfLines={1}>
                  {item.name}
                </Text>
                {item.size != null && item.kind === 'file' ? (
                  <Text style={{ color: colors.sub, fontSize: 12 }}>{sizeLabel(item.size)}</Text>
                ) : null}
                {state ? (
                  <Text
                    style={{
                      color:
                        state === 'deleted' || state === 'conflict'
                          ? colors.dangerInk
                          : state === 'untracked'
                            ? colors.sub
                            : colors.text,
                      fontWeight: '700',
                      width: 16,
                    }}
                  >
                    {STATE_MARK[state] ?? ''}
                  </Text>
                ) : null}
              </Pressable>
            );
          }}
        />
      ) : (
        <FlatList
          data={changed}
          keyExtractor={(f, i) => `${f.path}-${i}`}
          ListEmptyComponent={<Empty text={t('Nothing here.')} />}
          renderItem={({ item }) => (
            <Pressable
              onPress={() => open(item.path, { scope })}
              accessibilityRole="button"
              style={{
                minHeight: 52,
                paddingHorizontal: 16,
                flexDirection: 'row',
                alignItems: 'center',
                gap: 10,
                borderBottomWidth: 1,
                borderBottomColor: colors.line,
                backgroundColor: colors.card,
              }}
            >
              <Text style={{ color: colors.text, flex: 1 }} numberOfLines={1}>
                {item.path}
              </Text>
              <Text style={{ color: colors.text }}>+{item.added}</Text>
              <Text style={{ color: colors.sub }}>−{item.removed}</Text>
            </Pressable>
          )}
        />
      )}
    </View>
  );
}
