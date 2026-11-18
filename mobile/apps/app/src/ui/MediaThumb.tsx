import React, { useState } from 'react';
import { Pressable, Text, View } from 'react-native';
import { Image } from 'expo-image';
import { useRouter } from 'expo-router';
import { sourceId, type Resolved } from '@orochi/core';
import { sourceUrl } from '../lib/media';
import { useSession } from '../state/session';
import { usePrefs } from '../state/prefs';
import { Icon } from './Icon';
import { AudioRow } from './Players';

/** One media link in a message: an inline image, a video tile or an audio row. */
export function MediaThumb({ resolved, thread }: { resolved: Resolved; thread: string | null }) {
  const { client } = useSession();
  const { colors, t } = usePrefs();
  const router = useRouter();
  const [failed, setFailed] = useState(false);
  const [ratio, setRatio] = useState(1.5);
  if (!resolved.source || !resolved.media) return null;
  const url = sourceUrl(client, resolved.source, thread ?? '');
  const open = () =>
    router.push({
      pathname: '/viewer/media',
      params: { thread: thread ?? '', id: sourceId(resolved.source!) },
    });
  const name = resolved.label;
  if (!url || failed)
    return (
      <Text style={{ color: colors.sub, fontSize: 13 }}>
        {name} · {t('Preview unavailable')}
      </Text>
    );
  if (resolved.media === 'image')
    return (
      <Pressable
        onPress={open}
        accessibilityRole="imagebutton"
        accessibilityLabel={name}
        style={{ alignSelf: 'flex-start', maxWidth: '100%' }}
      >
        <Image
          source={{ uri: url }}
          style={{
            width: 280,
            maxWidth: '100%',
            aspectRatio: ratio,
            borderRadius: 10,
            backgroundColor: colors.code,
          }}
          contentFit="cover"
          onLoad={(e) =>
            e.source.width &&
            setRatio(Math.min(2.5, Math.max(0.5, e.source.width / e.source.height)))
          }
          onError={() => setFailed(true)}
        />
      </Pressable>
    );
  if (resolved.media === 'audio') return <AudioRow url={url} name={name} />;
  return (
    <Pressable
      onPress={open}
      accessibilityRole="button"
      accessibilityLabel={name}
      style={{
        width: 280,
        maxWidth: '100%',
        height: 158,
        borderRadius: 10,
        backgroundColor: '#111',
        alignItems: 'center',
        justifyContent: 'center',
      }}
    >
      <Icon name="play-circle" size={52} color="#fff" />
      <Text style={{ color: '#ddd', marginTop: 6, fontSize: 12 }} numberOfLines={1}>
        {name}
      </Text>
    </Pressable>
  );
}

export function MediaCard({
  url,
  kind,
  name,
  kept,
  onOpen,
}: {
  url: string;
  kind: string;
  name: string;
  kept: boolean;
  onOpen: () => void;
}) {
  const { colors, t } = usePrefs();
  if (!kept) return <Text style={{ color: colors.sub }}>{t('no longer kept')}</Text>;
  if (kind === 'audio') return <AudioRow url={url} name={name} />;
  return (
    <Pressable onPress={onOpen} style={{ alignSelf: 'flex-start' }} accessibilityLabel={name}>
      {kind === 'image' ? (
        <Image
          source={{ uri: url }}
          style={{
            width: 280,
            maxWidth: '100%',
            aspectRatio: 1.5,
            borderRadius: 10,
            backgroundColor: colors.code,
          }}
          contentFit="cover"
        />
      ) : (
        <View
          style={{
            width: 280,
            height: 158,
            borderRadius: 10,
            backgroundColor: '#111',
            alignItems: 'center',
            justifyContent: 'center',
          }}
        >
          <Icon name="play-circle" size={52} color="#fff" />
        </View>
      )}
      <Text style={{ color: colors.sub, fontSize: 12, marginTop: 3 }}>{name}</Text>
    </Pressable>
  );
}
