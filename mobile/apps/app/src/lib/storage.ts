import { Platform } from 'react-native';
import * as SecureStore from 'expo-secure-store';

// The token lives in the keychain/keystore. The web build (used only to try the UI in a
// browser) has no secure store, so it falls back to localStorage there and nowhere else.
export async function getSecret(key: string): Promise<string | null> {
  try {
    if (Platform.OS === 'web') return globalThis.localStorage?.getItem(key) ?? null;
    return await SecureStore.getItemAsync(key);
  } catch {
    return null;
  }
}

export async function setSecret(key: string, value: string | null): Promise<void> {
  try {
    if (Platform.OS === 'web') {
      if (value === null) globalThis.localStorage?.removeItem(key);
      else globalThis.localStorage?.setItem(key, value);
      return;
    }
    if (value === null) await SecureStore.deleteItemAsync(key);
    else await SecureStore.setItemAsync(key, value);
  } catch {
    /* a store that cannot be written leaves the app unpaired next launch */
  }
}
