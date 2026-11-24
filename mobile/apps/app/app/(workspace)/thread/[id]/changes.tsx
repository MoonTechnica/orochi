import React from 'react';
import { FlatList, Pressable, Text, View } from 'react-native';
import { useLocalSearchParams, useRouter } from 'expo-router';
import { usePrefs } from '../../../../src/state/prefs';
import { useThreadData } from '../../../../src/state/thread';
import { Empty, Header } from '../../../../src/ui/Screen';

export default function Changes() {
  const { id } = useLocalSearchParams<{ id: string }>();
  const { colors, t } = usePrefs();
  const router = useRouter();
  const { thread } = useThreadData(id);
  const files = thread?.files ?? [];
  return (
    <View style={{ flex: 1, backgroundColor: colors.bg }}>
      <Header title={t('Changes')} subtitle={thread?.thread.title} />
      {files.length === 0 ? <Empty text={t('Nothing here.')} /> : null}
      <FlatList
        data={files}
        keyExtractor={(f, i) => `${f.path}-${i}`}
        renderItem={({ item }) => (
          <Pressable
            onPress={() =>
              router.push({
                pathname: '/viewer/file',
                params: { thread: id, path: item.path, patch: String(item.latest_patch) },
              })
            }
            style={{
              minHeight: 56,
              paddingHorizontal: 16,
              paddingVertical: 10,
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
    </View>
  );
}
