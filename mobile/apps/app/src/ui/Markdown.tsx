import React, { useMemo } from 'react';
import { Linking, Pressable, ScrollView, StyleSheet, Text, View } from 'react-native';
import { useRouter } from 'expo-router';
import {
  parseMarkdown,
  resolveLink,
  sourceId,
  type Block,
  type Inline,
  type Resolved,
} from '@orochi/core';
import { usePrefs } from '../state/prefs';
import { MediaThumb } from './MediaThumb';
import { mono } from './theme';

export interface MdContext {
  thread: string | null;
  cwd: string | null | undefined;
  known?: Set<string>;
  mentions?: string[];
}

export function Markdown({
  text,
  ctx,
  size = 15,
}: {
  text: string;
  ctx: MdContext;
  size?: number;
}) {
  const blocks = useMemo(() => parseMarkdown(text, ctx.known), [text, ctx.known]);
  return (
    <View style={{ gap: 6 }}>
      {blocks.map((b, i) => (
        <BlockView key={i} block={b} ctx={ctx} size={size} />
      ))}
    </View>
  );
}

function BlockView({ block, ctx, size }: { block: Block; ctx: MdContext; size: number }) {
  const { colors } = usePrefs();
  switch (block.t) {
    case 'p': {
      const media: Resolved[] = [];
      return (
        <View style={{ gap: 6 }}>
          <Text style={{ color: colors.text, fontSize: size, lineHeight: size * 1.4 }} selectable>
            {block.lines.map((line, n) => (
              <React.Fragment key={n}>
                {n > 0 ? '\n' : null}
                <Inlines nodes={line} ctx={ctx} size={size} media={media} />
              </React.Fragment>
            ))}
          </Text>
          {media.map((m, i) => (
            <MediaThumb key={`${sourceId(m.source!)}-${i}`} resolved={m} thread={ctx.thread} />
          ))}
        </View>
      );
    }
    case 'heading': {
      const media: Resolved[] = [];
      return (
        <Text
          style={{
            color: colors.text,
            fontSize: size + 6 - Math.min(block.level, 4) * 1.5,
            fontWeight: '700',
            marginTop: 4,
          }}
        >
          <Inlines nodes={block.inline} ctx={ctx} size={size} media={media} />
        </Text>
      );
    }
    case 'code':
      return (
        <ScrollView
          horizontal
          style={[styles.code, { backgroundColor: colors.code }]}
          showsHorizontalScrollIndicator={false}
        >
          <Text selectable style={{ fontFamily: mono, fontSize: size - 2, color: colors.text }}>
            {block.text}
          </Text>
        </ScrollView>
      );
    case 'list': {
      const media: Resolved[] = [];
      return (
        <View style={{ gap: 3 }}>
          {block.items.map((item, i) => (
            <View key={i} style={{ flexDirection: 'row', gap: 6 }}>
              <Text style={{ color: colors.sub, fontSize: size }}>
                {block.ordered ? `${i + 1}.` : '•'}
              </Text>
              <Text
                style={{ color: colors.text, fontSize: size, lineHeight: size * 1.4, flex: 1 }}
                selectable
              >
                <Inlines nodes={item} ctx={ctx} size={size} media={media} />
              </Text>
            </View>
          ))}
          {media.map((m, i) => (
            <MediaThumb key={i} resolved={m} thread={ctx.thread} />
          ))}
        </View>
      );
    }
    case 'quote':
      return (
        <View style={{ borderLeftWidth: 3, borderLeftColor: colors.line, paddingLeft: 10 }}>
          {block.blocks.map((b, i) => (
            <BlockView key={i} block={b} ctx={ctx} size={size} />
          ))}
        </View>
      );
    case 'rule':
      return (
        <View
          style={{
            height: StyleSheet.hairlineWidth,
            backgroundColor: colors.line,
            marginVertical: 4,
          }}
        />
      );
    case 'table': {
      const media: Resolved[] = [];
      const row = (cells: Inline[][], head: boolean, k: number) => (
        <View key={k} style={{ flexDirection: 'row' }}>
          {cells.map((c, i) => (
            <Text
              key={i}
              style={{
                minWidth: 90,
                padding: 6,
                color: colors.text,
                fontWeight: head ? '700' : '400',
                fontSize: size - 1,
                borderWidth: StyleSheet.hairlineWidth,
                borderColor: colors.line,
              }}
            >
              <Inlines nodes={c} ctx={ctx} size={size} media={media} />
            </Text>
          ))}
        </View>
      );
      return (
        <ScrollView horizontal showsHorizontalScrollIndicator={false}>
          <View>
            {row(block.head, true, -1)}
            {block.rows.map((r, i) => row(r, false, i))}
          </View>
        </ScrollView>
      );
    }
  }
}

function Inlines({
  nodes,
  ctx,
  size,
  media,
}: {
  nodes: Inline[];
  ctx: MdContext;
  size: number;
  media: Resolved[];
}) {
  const { colors } = usePrefs();
  const router = useRouter();
  return (
    <>
      {nodes.map((n, i) => {
        switch (n.t) {
          case 'text':
            return n.text;
          case 'code':
            return (
              <Text
                key={i}
                style={{ fontFamily: mono, backgroundColor: colors.code, fontSize: size - 2 }}
              >
                {` ${n.text} `}
              </Text>
            );
          case 'strong':
            return (
              <Text key={i} style={{ fontWeight: '700' }}>
                <Inlines nodes={n.children} ctx={ctx} size={size} media={media} />
              </Text>
            );
          case 'em':
            return (
              <Text key={i} style={{ fontStyle: 'italic' }}>
                <Inlines nodes={n.children} ctx={ctx} size={size} media={media} />
              </Text>
            );
          case 'del':
            return (
              <Text key={i} style={{ textDecorationLine: 'line-through' }}>
                <Inlines nodes={n.children} ctx={ctx} size={size} media={media} />
              </Text>
            );
          case 'mention': {
            const you = n.name.toLowerCase() === 'you';
            return (
              <Text
                key={i}
                style={{
                  color: colors.text,
                  fontWeight: '600',
                  backgroundColor: you ? colors.mark : undefined,
                }}
              >
                {n.text}
              </Text>
            );
          }
          case 'link': {
            const r = resolveLink(n.label, n.target, n.image, ctx);
            if (r.media && r.source) {
              media.push(r);
              if (n.image) return null;
              return (
                <Text key={i} style={{ color: colors.text }}>
                  {r.label}
                </Text>
              );
            }
            if (r.source?.type === 'file') {
              const source = r.source;
              return (
                <Text
                  key={i}
                  style={{ color: colors.text, textDecorationLine: 'underline' }}
                  onPress={() =>
                    router.push({
                      pathname: '/viewer/file',
                      params: {
                        thread: ctx.thread ?? '',
                        path: source.path,
                        line: String(source.line ?? ''),
                      },
                    })
                  }
                >
                  {r.label}
                </Text>
              );
            }
            if (r.source?.type === 'url') {
              const url = r.source.url;
              return (
                <Text
                  key={i}
                  style={{ color: colors.text, textDecorationLine: 'underline' }}
                  onPress={() => Linking.openURL(url).catch(() => {})}
                >
                  {r.label}
                </Text>
              );
            }
            return <Text key={i}>{r.label}</Text>;
          }
        }
      })}
    </>
  );
}

const styles = StyleSheet.create({
  code: { borderRadius: 8, padding: 10, maxHeight: 360 },
});
export { Pressable };
