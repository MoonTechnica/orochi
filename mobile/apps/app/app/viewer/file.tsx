import React, { useEffect, useRef, useState } from 'react';
import { ActivityIndicator, FlatList, Pressable, ScrollView, Text, View } from 'react-native';
import { Image } from 'expo-image';
import { useLocalSearchParams } from 'expo-router';
import { fileKind } from '@orochi/core';
import type { DiffScope, FileText } from '@orochi/protocol';
import { usePrefs } from '../../src/state/prefs';
import { useSession } from '../../src/state/session';
import { DiffView } from '../../src/ui/DiffView';
import { AudioRow, VideoPlayerView } from '../../src/ui/Players';
import { Empty, Header } from '../../src/ui/Screen';
import { TextPrompt } from '../../src/ui/Sheet';
import { mono } from '../../src/ui/theme';

/**
 * Text, code, images, video, audio and diffs. `patch` shows one recorded change, `scope` the
 * working tree's diff for the path (where review comments can be written), `commit` a commit.
 */
export default function FileViewer() {
  const { thread, path, line, patch, scope, commit } = useLocalSearchParams<{
    thread: string;
    path?: string;
    line?: string;
    patch?: string;
    scope?: string;
    commit?: string;
  }>();
  const { api, client } = useSession();
  const { colors, t } = usePrefs();
  const [file, setFile] = useState<FileText | null>(null);
  const [diff, setDiff] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [showDiff, setShowDiff] = useState(!!(patch || scope || commit));
  const [asking, setAsking] = useState<{ line: number; text: string } | null>(null);
  const [review, setReview] = useState<[string, number, string][]>([]);
  const [sent, setSent] = useState(false);
  const list = useRef<FlatList>(null);
  const name = path ?? commit?.slice(0, 8) ?? '';
  const kind = path ? fileKind(path) : 'text';

  useEffect(() => {
    if (!api) return;
    (async () => {
      try {
        if (commit) setDiff(await api.gitShow(thread, commit));
        else if (patch) setDiff(await api.patch(Number(patch)));
        else if (scope && path) setDiff(await api.treePatch(thread, scope, path));
        if (path && kind === 'text' && !commit) setFile(await api.treeRead(thread, path));
      } catch (e: any) {
        setError(String(e?.message ?? e));
      }
    })();
  }, [api, thread, path, patch, scope, commit, kind]);

  const url = client && path ? client.fileUrl(thread, path) : null;
  const text = file?.text ?? '';
  const lines = text.split('\n');
  const target = Number(line) || 0;
  const hasDiff = !!diff;
  const canReview = !!scope && !!path;
  const send = async () => {
    if (!api || review.length === 0) return;
    await api.comment(thread, review);
    await api.ensureHost(thread).catch(() => {});
    setReview([]);
    setSent(true);
  };

  return (
    <View style={{ flex: 1, backgroundColor: colors.bg }}>
      <Header title={name.split('/').pop() ?? name} subtitle={commit ? t('Commit') : path} />
      {error ? <Empty text={error} /> : null}
      {!error && !file && !diff && kind === 'text' ? (
        <ActivityIndicator style={{ margin: 24 }} color={colors.text} />
      ) : null}
      {kind === 'image' && url ? (
        <Image source={{ uri: url }} style={{ flex: 1 }} contentFit="contain" />
      ) : null}
      {kind === 'video' && url ? <VideoPlayerView url={url} active /> : null}
      {kind === 'audio' && url ? (
        <View style={{ padding: 16 }}>
          <AudioRow url={url} name={name} />
        </View>
      ) : null}
      {file?.binary ? <Empty text={t('Binary file')} /> : null}
      {file?.truncated ? (
        <Text style={{ color: colors.text, padding: 8, fontSize: 12 }}>{t('Truncated')}</Text>
      ) : null}
      {showDiff && hasDiff ? (
        <DiffView
          text={diff!}
          onLine={canReview ? (n, tx) => setAsking({ line: n, text: tx }) : undefined}
          marked={new Set(review.map((r) => r[1]))}
        />
      ) : null}
      {showDiff && !hasDiff && (patch || scope || commit) && !error && diff === null ? null : null}
      {!showDiff && file && !file.binary && kind === 'text' ? (
        <ScrollView horizontal contentContainerStyle={{ flexGrow: 1 }}>
          <FlatList
            ref={list}
            data={lines}
            keyExtractor={(_, i) => String(i)}
            initialNumToRender={60}
            onLayout={() =>
              target > 0 &&
              list.current?.scrollToIndex({
                index: Math.min(lines.length - 1, target - 1),
                animated: false,
              })
            }
            onScrollToIndexFailed={() => {}}
            renderItem={({ item, index }) => (
              <View
                style={{
                  flexDirection: 'row',
                  backgroundColor: index + 1 === target ? colors.mark : undefined,
                }}
              >
                <Text
                  style={{
                    width: 44,
                    textAlign: 'right',
                    paddingRight: 8,
                    color: colors.sub,
                    fontFamily: mono,
                    fontSize: 12,
                  }}
                >
                  {index + 1}
                </Text>
                <Text
                  selectable
                  style={{ fontFamily: mono, fontSize: 12, color: colors.text, paddingRight: 16 }}
                >
                  {item || ' '}
                </Text>
              </View>
            )}
          />
        </ScrollView>
      ) : null}
      {canReview && showDiff ? (
        <Text style={{ color: colors.sub, fontSize: 12, padding: 8 }}>
          {sent ? t('Review sent.') : t('Tap a line to comment on it.')}
        </Text>
      ) : null}
      {review.length > 0 ? (
        <Pressable
          onPress={send}
          accessibilityRole="button"
          style={{
            margin: 12,
            minHeight: 48,
            borderRadius: 12,
            backgroundColor: colors.primary,
            alignItems: 'center',
            justifyContent: 'center',
          }}
        >
          <Text style={{ color: colors.onPrimary, fontWeight: '700', fontSize: 16 }}>
            {t('Send review')} ({review.length})
          </Text>
        </Pressable>
      ) : null}
      {hasDiff && file ? (
        <Pressable
          onPress={() => setShowDiff(!showDiff)}
          style={{ minHeight: 44, alignItems: 'center', justifyContent: 'center' }}
        >
          <Text style={{ color: colors.text }}>{showDiff ? t('Open file') : t('Changes')}</Text>
        </Pressable>
      ) : null}
      <TextPrompt
        visible={!!asking}
        title={`${t('Comment')} · ${path}:${asking?.line ?? ''}`}
        confirm={t('Add')}
        onClose={() => setAsking(null)}
        onSubmit={(note) => {
          if (note && asking && path) setReview((r) => [...r, [path, asking.line, note]]);
        }}
      />
    </View>
  );
}
export type { DiffScope };
