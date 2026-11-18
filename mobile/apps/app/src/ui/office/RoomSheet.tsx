import React from 'react';
import { FlatList, Modal, Pressable, Text, View } from 'react-native';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import { listPeople, type Person, type Room } from '@orochi/core';
import { usePrefs } from '../../state/prefs';
import { Avatar } from './Avatar';
import { nameOf, stateLabel } from './Seat';

/** Everyone in one room as a compact list, for when the room shows only the first few. */
export function RoomSheet({
  room,
  icons,
  onClose,
  onPerson,
}: {
  room: Room | null;
  icons: Record<string, string>;
  onClose: () => void;
  onPerson: (p: Person) => void;
}) {
  const { colors, t } = usePrefs();
  const insets = useSafeAreaInsets();
  const people = React.useMemo(() => (room ? listPeople(room.people) : []), [room]);
  const title = room ? room.title || t('New conversation') : '';
  return (
    <Modal visible={!!room} transparent animationType="fade" onRequestClose={onClose}>
      <View
        accessibilityViewIsModal
        style={{ flex: 1, backgroundColor: '#0006', justifyContent: 'flex-end' }}
      >
        <Pressable
          style={{ position: 'absolute', top: 0, bottom: 0, left: 0, right: 0 }}
          onPress={onClose}
          accessibilityRole="button"
          accessibilityLabel={t('Close')}
        />
        {room ? (
          <View
            testID="room-sheet"
            style={{
              backgroundColor: colors.card,
              borderTopLeftRadius: 12,
              borderTopRightRadius: 12,
              paddingTop: 18,
              paddingBottom: insets.bottom + 14,
              gap: 10,
              maxHeight: '75%',
            }}
          >
            <View style={{ paddingHorizontal: 18, gap: 2 }}>
              <Text
                numberOfLines={1}
                style={{ color: colors.text, fontSize: 18, fontWeight: '800' }}
              >
                {t('Everyone in {room}').replace('{room}', title)}
              </Text>
              <Text style={{ color: colors.sub, fontSize: 13 }}>
                {people.length} {t(people.length === 1 ? 'person' : 'people')}
              </Text>
            </View>
            <FlatList
              style={{ flexGrow: 0, flexShrink: 1 }}
              data={people}
              keyExtractor={(p, i) => `${p.key}:${i}`}
              initialNumToRender={12}
              windowSize={5}
              contentContainerStyle={{ paddingHorizontal: 12 }}
              renderItem={({ item: p }) => (
                <Pressable
                  testID={`member-${p.key}`}
                  accessibilityRole="button"
                  accessibilityLabel={[
                    nameOf(p, t),
                    [p.agent, p.model].filter(Boolean).join(' '),
                    stateLabel(p, t),
                    p.doing,
                  ]
                    .filter(Boolean)
                    .join(', ')}
                  onPress={() => onPerson(p)}
                  style={{
                    flexDirection: 'row',
                    alignItems: 'center',
                    gap: 12,
                    minHeight: 52,
                    paddingVertical: 6,
                    paddingHorizontal: 6,
                    borderBottomWidth: 1,
                    borderBottomColor: colors.line,
                  }}
                >
                  <Avatar person={p} size={32} icon={icons[p.provider]} still />
                  <View style={{ flex: 1 }}>
                    <Text
                      numberOfLines={1}
                      style={{ color: colors.text, fontSize: 14, fontWeight: '700' }}
                    >
                      {nameOf(p, t)}
                      <Text style={{ color: colors.muted, fontWeight: '400', fontSize: 12 }}>
                        {'  '}
                        {[p.agent, p.model].filter(Boolean).join(' · ')}
                      </Text>
                    </Text>
                    <Text numberOfLines={1} style={{ color: colors.sub, fontSize: 12 }}>
                      <Text style={{ color: colors.text, fontWeight: '600' }}>
                        {stateLabel(p, t)}
                      </Text>
                      {p.doing ? ` · ${p.doing}` : ''}
                    </Text>
                  </View>
                </Pressable>
              )}
            />
            <Pressable
              accessibilityRole="button"
              accessibilityLabel={t('Close')}
              onPress={onClose}
              style={{
                marginHorizontal: 18,
                minHeight: 46,
                borderRadius: 10,
                borderWidth: 1,
                borderColor: colors.line,
                alignItems: 'center',
                justifyContent: 'center',
              }}
            >
              <Text style={{ color: colors.text, fontSize: 16, fontWeight: '700' }}>
                {t('Close')}
              </Text>
            </Pressable>
          </View>
        ) : null}
      </View>
    </Modal>
  );
}
