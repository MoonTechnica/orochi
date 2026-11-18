import React, { useEffect } from 'react';
import { Pressable, Text, View } from 'react-native';
import { useAudioPlayer, useAudioPlayerStatus } from 'expo-audio';
import { useVideoPlayer, VideoView } from 'expo-video';
import { usePrefs } from '../state/prefs';
import { Icon } from './Icon';

function fmt(s: number) {
  const m = Math.floor(s / 60);
  return `${m}:${String(Math.floor(s % 60)).padStart(2, '0')}`;
}

/** An audio file with play/pause and position; the player is only created once shown. */
export function AudioRow({ url, name }: { url: string; name: string }) {
  const { colors } = usePrefs();
  const player = useAudioPlayer({ uri: url });
  const status = useAudioPlayerStatus(player);
  useEffect(() => () => player.pause(), [player]);
  return (
    <View
      style={{
        flexDirection: 'row',
        alignItems: 'center',
        gap: 10,
        backgroundColor: colors.code,
        borderRadius: 12,
        padding: 8,
        width: 280,
        maxWidth: '100%',
      }}
    >
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={status.playing ? 'Pause' : 'Play'}
        onPress={() => {
          if (status.playing) player.pause();
          else {
            if (
              status.didJustFinish ||
              (status.duration > 0 && status.currentTime >= status.duration)
            )
              player.seekTo(0);
            player.play();
          }
        }}
        style={{ width: 44, height: 44, alignItems: 'center', justifyContent: 'center' }}
      >
        <Icon
          name={status.playing ? 'pause-circle' : 'play-circle'}
          size={36}
          color={colors.text}
        />
      </Pressable>
      <View style={{ flex: 1 }}>
        <Text numberOfLines={1} style={{ color: colors.text, fontSize: 14 }}>
          {name}
        </Text>
        <Text style={{ color: colors.sub, fontSize: 12 }}>
          {fmt(status.currentTime || 0)} / {fmt(Math.ceil(status.duration || 0))}
        </Text>
      </View>
    </View>
  );
}

export function VideoPlayerView({ url, active }: { url: string; active: boolean }) {
  const player = useVideoPlayer({ uri: url }, (p) => {
    p.loop = false;
  });
  useEffect(() => {
    if (!active) player.pause();
  }, [active, player]);
  return (
    <VideoView
      player={player}
      style={{ width: '100%', height: '100%' }}
      nativeControls
      contentFit="contain"
    />
  );
}
