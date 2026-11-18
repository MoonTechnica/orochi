import React, { useMemo } from 'react';
import { FlatList, Pressable, Text, View, useWindowDimensions } from 'react-native';
import { useLocalSearchParams, useRouter } from 'expo-router';
import { Image } from 'expo-image';
import { galleryOf } from '@orochi/core';
import { sourceUrl } from '../../../src/lib/media';
import { usePrefs } from '../../../src/state/prefs';
import { useSession } from '../../../src/state/session';
import { useThreadData } from '../../../src/state/thread';
import { Icon } from '../../../src/ui/Icon';
import { Empty, Header } from '../../../src/ui/Screen';

export default function Gallery() {
  const { id } = useLocalSearchParams<{ id: string }>();
  const { client } = useSession();
  const { colors, t } = usePrefs();
  const router = useRouter();
  const { width } = useWindowDimensions();
  const { thread } = useThreadData(id);
  const items = useMemo(
    () => (thread ? galleryOf(thread.items, { thread: id, cwd: thread.thread.cwd }) : []),
    [thread, id],
  );
  const cell = (width - 4) / 3;
  return (
    <View style={{ flex: 1, backgroundColor: colors.bg }}>
      <Header title={t('Gallery')} subtitle={thread?.thread.title} />
      {items.length === 0 ? <Empty text={t('No media in this conversation.')} /> : null}
      <FlatList
        data={items}
        numColumns={3}
        keyExtractor={(g) => g.id}
        columnWrapperStyle={{ gap: 2 }}
        contentContainerStyle={{ gap: 2 }}
        renderItem={({ item }) => {
          const url = sourceUrl(client, item.source, id);
          return (
            <Pressable
              onPress={() =>
                router.push({ pathname: '/viewer/media', params: { thread: id, id: item.id } })
              }
              accessibilityRole="imagebutton"
              accessibilityLabel={item.name}
              style={{
                width: cell,
                height: cell,
                backgroundColor: colors.code,
                alignItems: 'center',
                justifyContent: 'center',
              }}
            >
              {item.kind === 'image' && url ? (
                <Image
                  source={{ uri: url }}
                  style={{ width: cell, height: cell }}
                  contentFit="cover"
                />
              ) : (
                <>
                  <Icon
                    name={item.kind === 'video' ? 'videocam' : 'musical-notes'}
                    size={32}
                    color={colors.sub}
                  />
                  <Text
                    numberOfLines={1}
                    style={{ color: colors.sub, fontSize: 11, paddingHorizontal: 4 }}
                  >
                    {item.name}
                  </Text>
                </>
              )}
            </Pressable>
          );
        }}
      />
    </View>
  );
}
