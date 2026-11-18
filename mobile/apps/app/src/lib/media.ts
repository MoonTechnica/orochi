import { Platform, Linking } from 'react-native';
import type { RemoteClient } from '@orochi/protocol';
import type { MediaSource } from '@orochi/core';
import { File, Paths } from 'expo-file-system';
import * as Sharing from 'expo-sharing';
import * as MediaLibrary from 'expo-media-library';

export function sourceUrl(
  client: RemoteClient | null,
  source: MediaSource,
  thread: string,
): string | null {
  if (source.type === 'url') return source.url;
  if (!client) return null;
  if (source.type === 'file') return client.fileUrl(thread, source.path);
  return client.mediaUrl(source.thread, source.sha);
}

async function download(url: string, name: string): Promise<string> {
  const safe = name.replace(/[^\w.-]+/g, '_') || 'media';
  const target = new File(Paths.cache, `${Date.now()}-${safe}`);
  const file = await File.downloadFileAsync(url, target);
  return file.uri;
}

export async function shareMedia(url: string, name: string): Promise<'shared' | 'unsupported'> {
  if (Platform.OS === 'web') {
    await Linking.openURL(url);
    return 'shared';
  }
  const uri = await download(url, name);
  if (!(await Sharing.isAvailableAsync())) return 'unsupported';
  await Sharing.shareAsync(uri);
  return 'shared';
}

export type SaveResult = 'saved' | 'denied' | 'unsupported';

export async function saveMedia(url: string, name: string): Promise<SaveResult> {
  if (Platform.OS === 'web') {
    await Linking.openURL(url);
    return 'unsupported';
  }
  const permission = await MediaLibrary.requestPermissionsAsync(true);
  if (!permission.granted) return 'denied';
  const uri = await download(url, name);
  await MediaLibrary.Asset.create(uri);
  return 'saved';
}
