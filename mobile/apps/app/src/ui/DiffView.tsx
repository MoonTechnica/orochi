import React, { useMemo } from 'react';
import { FlatList, Pressable, ScrollView, Text, View } from 'react-native';
import { parseDiff } from '@orochi/core';
import { usePrefs } from '../state/prefs';
import { mono } from './theme';

/** A unified diff; a tap on an added or context line names it (for review comments). */
export function DiffView({
  text,
  onLine,
  marked,
}: {
  text: string;
  onLine?: (line: number, text: string) => void;
  marked?: Set<number>;
}) {
  const { colors } = usePrefs();
  const lines = useMemo(() => parseDiff(text), [text]);
  return (
    <ScrollView horizontal contentContainerStyle={{ flexGrow: 1 }}>
      <FlatList
        data={lines}
        keyExtractor={(_, i) => String(i)}
        initialNumToRender={80}
        renderItem={({ item }) => {
          const bg =
            item.kind === 'add'
              ? colors.text + '14'
              : item.kind === 'del'
                ? colors.sub + '33'
                : item.kind === 'hunk'
                  ? colors.line
                  : undefined;
          const tappable = !!onLine && item.line !== null;
          return (
            <Pressable
              disabled={!tappable}
              onPress={() => onLine!(item.line!, item.text.slice(1))}
              accessibilityLabel={tappable ? `line ${item.line}` : undefined}
              style={{
                flexDirection: 'row',
                backgroundColor: marked?.has(item.line ?? -1) ? colors.mark : bg,
              }}
            >
              <Text
                style={{
                  width: 40,
                  textAlign: 'right',
                  paddingRight: 8,
                  color: colors.sub,
                  fontFamily: mono,
                  fontSize: 11,
                }}
              >
                {item.line ?? ''}
              </Text>
              <Text
                style={{
                  fontFamily: mono,
                  fontSize: 12,
                  color:
                    item.kind === 'hunk'
                      ? colors.primary
                      : item.kind === 'meta'
                        ? colors.sub
                        : colors.text,
                  paddingRight: 16,
                }}
              >
                {item.text || ' '}
              </Text>
            </Pressable>
          );
        }}
      />
    </ScrollView>
  );
}
