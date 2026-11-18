import React from 'react';
import { Pressable, Text, View } from 'react-native';
import { STATUS_TONE, since, type Trail as TrailGroup } from '@orochi/core';
import { usePrefs } from '../../state/prefs';
import { toneFill } from '../theme';

const RAIL = 22;

/** Recent work as branches of a rail: a project is a branch, a conversation a node on it. */
export function Trail({
  groups,
  many,
  onOpen,
}: {
  groups: TrailGroup[];
  many: boolean;
  onOpen: (machineKey: string, thread: string) => void;
}) {
  const { colors, t } = usePrefs();
  const tone = (s: keyof typeof STATUS_TONE) => toneFill(colors, STATUS_TONE[s]);
  return (
    <View style={{ paddingHorizontal: 18, gap: 18 }}>
      {groups.map((g) => (
        <View key={g.key} testID={`branch-${g.project}`}>
          <View style={{ flexDirection: 'row', alignItems: 'center', height: 28 }}>
            <View style={{ width: RAIL, alignItems: 'flex-start' }}>
              <View
                style={{
                  width: 14,
                  height: 14,
                  borderRadius: 3,
                  transform: [{ rotate: '45deg' }],
                  backgroundColor: colors.primary,
                }}
              />
            </View>
            <Text
              numberOfLines={1}
              style={{ color: colors.text, fontWeight: '700', fontSize: 18, flexShrink: 1 }}
            >
              {g.project || t('New conversation')}
            </Text>
            {many ? (
              <Text numberOfLines={1} style={{ color: colors.sub, fontSize: 12, marginLeft: 8 }}>
                {g.machineName}
              </Text>
            ) : null}
          </View>
          <View>
            <View
              style={{
                position: 'absolute',
                left: 6,
                top: 0,
                bottom: 14,
                width: 2,
                borderRadius: 1,
                backgroundColor: colors.line,
              }}
            />
            {g.items.map((r) => {
              const live = r.thread.status === 'working' || r.thread.status === 'background';
              const ask = r.thread.status === 'needs_you';
              const c = tone(r.thread.status);
              return (
                <Pressable
                  key={`${r.machineKey}:${r.thread.id}`}
                  accessibilityRole="button"
                  accessibilityLabel={`${r.thread.title || t('New conversation')}, ${since(r.thread.updated_at)}`}
                  onPress={() => onOpen(r.machineKey, r.thread.id)}
                  style={{ minHeight: 46, flexDirection: 'row', alignItems: 'center' }}
                >
                  <View style={{ width: RAIL, alignItems: 'flex-start' }}>
                    <View
                      style={{
                        width: 14,
                        height: 14,
                        borderRadius: 7,
                        marginLeft: 0,
                        backgroundColor: live || ask ? c : colors.bg,
                        borderWidth: 2,
                        borderColor: live || ask ? c : colors.muted,
                      }}
                    />
                  </View>
                  <Text
                    numberOfLines={1}
                    style={{
                      color: colors.text,
                      fontSize: 15,
                      fontWeight: ask ? '800' : '500',
                      flex: 1,
                    }}
                  >
                    {r.thread.pinned ? '◆ ' : ''}
                    {r.thread.title || t('New conversation')}
                  </Text>
                  <Text style={{ color: colors.sub, fontSize: 12, marginLeft: 8 }}>
                    {since(r.thread.updated_at)}
                  </Text>
                </Pressable>
              );
            })}
          </View>
        </View>
      ))}
    </View>
  );
}
