import React, { useMemo, useRef, useState } from 'react';
import {
  Alert,
  FlatList,
  Platform,
  Pressable,
  Text,
  View,
  useWindowDimensions,
} from 'react-native';
import { useLocalSearchParams, useRouter } from 'expo-router';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import { galleryOf } from '@orochi/core';
import { saveMedia, shareMedia, sourceUrl } from '../../src/lib/media';
import { usePrefs } from '../../src/state/prefs';
import { useSession } from '../../src/state/session';
import { useThreadData } from '../../src/state/thread';
import { Icon } from '../../src/ui/Icon';
import { AudioRow, VideoPlayerView } from '../../src/ui/Players';
import { ZoomableImage } from '../../src/ui/Zoomable';

/** Full-screen media: swipe through the conversation's media, pinch to zoom, save or share. */
export default function MediaViewer() {
  const { thread: id, id: start } = useLocalSearchParams<{ thread: string; id: string }>();
  const { client } = useSession();
  const { t } = usePrefs();
  const router = useRouter();
  const insets = useSafeAreaInsets();
  const { width, height } = useWindowDimensions();
  const { thread } = useThreadData(id);
  const items = useMemo(
    () => (thread ? galleryOf(thread.items, { thread: id, cwd: thread.thread.cwd }) : []),
    [thread, id],
  );
  const initial = Math.max(
    0,
    items.findIndex((g) => g.id === start),
  );
  const [index, setIndex] = useState<number | null>(null);
  const [zoomed, setZoomed] = useState(false);
  const [chrome, setChrome] = useState(true);
  const [note, setNote] = useState<string | null>(null);
  const list = useRef<FlatList>(null);
  const current = index ?? initial;
  const item = items[current];
  const url = item ? sourceUrl(client, item.source, id) : null;

  const say = (message: string) => {
    setNote(message);
    setTimeout(() => setNote(null), 2500);
  };
  const save = async () => {
    if (!url || !item) return;
    try {
      const r = await saveMedia(url, item.name);
      say(
        r === 'saved'
          ? t('Saved to your library')
          : r === 'denied'
            ? t('Photo permission is needed to save.')
            : t('Could not save'),
      );
    } catch {
      say(t('Could not save'));
    }
  };
  const share = async () => {
    if (!url || !item) return;
    try {
      await shareMedia(url, item.name);
    } catch {
      if (Platform.OS !== 'web') Alert.alert(t('Could not save'));
    }
  };

  const button = (
    icon: React.ComponentProps<typeof Icon>['name'],
    label: string,
    onPress: () => void,
  ) => (
    <Pressable
      onPress={onPress}
      accessibilityRole="button"
      accessibilityLabel={label}
      style={{ width: 48, height: 48, alignItems: 'center', justifyContent: 'center' }}
    >
      <Icon name={icon} size={26} color="#fff" />
    </Pressable>
  );

  return (
    <View style={{ flex: 1, backgroundColor: '#000' }}>
      {items.length > 0 ? (
        <FlatList
          ref={list}
          data={items}
          horizontal
          pagingEnabled
          scrollEnabled={!zoomed}
          initialScrollIndex={initial}
          getItemLayout={(_, i) => ({ length: width, offset: width * i, index: i })}
          keyExtractor={(g) => g.id}
          showsHorizontalScrollIndicator={false}
          onMomentumScrollEnd={(e) => setIndex(Math.round(e.nativeEvent.contentOffset.x / width))}
          renderItem={({ item: g, index: i }) => {
            const u = sourceUrl(client, g.source, id);
            return (
              <View style={{ width, height, alignItems: 'center', justifyContent: 'center' }}>
                {u && g.kind === 'image' ? (
                  <ZoomableImage
                    url={u}
                    width={width}
                    height={height}
                    onZoomChange={setZoomed}
                    onTap={() => setChrome((c) => !c)}
                  />
                ) : null}
                {u && g.kind === 'video' ? (
                  <VideoPlayerView url={u} active={i === current} />
                ) : null}
                {u && g.kind === 'audio' ? <AudioRow url={u} name={g.name} /> : null}
              </View>
            );
          }}
        />
      ) : null}
      {chrome ? (
        <>
          <View
            style={{
              position: 'absolute',
              top: 0,
              left: 0,
              right: 0,
              paddingTop: insets.top,
              backgroundColor: '#000a',
              flexDirection: 'row',
              alignItems: 'center',
            }}
          >
            {button('close', t('Close'), () =>
              router.canGoBack() ? router.back() : router.replace('/'),
            )}
            <View style={{ flex: 1 }}>
              <Text numberOfLines={1} style={{ color: '#fff', fontWeight: '600' }}>
                {item?.name}
              </Text>
              <Text style={{ color: '#bbb', fontSize: 12 }}>
                {items.length ? `${current + 1}/${items.length}` : ''}
                {item?.mime ? ` · ${item.mime}` : ''}
              </Text>
            </View>
            {button('download-outline', t('Save'), save)}
            {button('share-outline', t('Share'), share)}
          </View>
          {note ? (
            <View
              style={{
                position: 'absolute',
                bottom: insets.bottom + 24,
                alignSelf: 'center',
                backgroundColor: '#000c',
                paddingHorizontal: 16,
                paddingVertical: 10,
                borderRadius: 10,
              }}
            >
              <Text style={{ color: '#fff' }}>{note}</Text>
            </View>
          ) : null}
        </>
      ) : null}
    </View>
  );
}
