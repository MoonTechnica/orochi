import { useEffect, useState } from 'react';
import { getSecret, setSecret } from '../lib/storage';

const KEY = 'orochi.you';
let seed: string | null = null;
const waiting = new Set<(s: string) => void>();
let loading = false;

function load() {
  if (loading || seed !== null) return;
  loading = true;
  getSecret(KEY).then(async (saved) => {
    let value = saved;
    if (!value) {
      value = Math.random().toString(36).slice(2, 10) || 'you';
      await setSecret(KEY, value);
    }
    seed = value;
    loading = false;
    waiting.forEach((f) => f(value!));
    waiting.clear();
  });
}

/**
 * The seed that picks the calm character standing for the person using this phone: made once,
 * kept per install, never sent anywhere. Empty until it is read (the first frame draws a default).
 */
export function useYouSeed(): string {
  const [value, setValue] = useState(seed ?? '');
  useEffect(() => {
    if (seed !== null) {
      setValue(seed);
      return;
    }
    waiting.add(setValue);
    load();
    return () => {
      waiting.delete(setValue);
    };
  }, []);
  return value;
}
