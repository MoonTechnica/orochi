import React, { createContext, useContext, useEffect, useMemo, useState } from 'react';
import { Platform, useColorScheme } from 'react-native';
import * as Localization from 'expo-localization';
import { detectLang, t as translate, type Lang } from '@orochi/core';
import { getSecret, setSecret } from '../lib/storage';
import { palette, type Colors } from '../ui/theme';

interface Prefs {
  lang: Lang;
  setLang(lang: Lang): void;
  colors: Colors;
  dark: boolean;
  t(key: string): string;
}

const Ctx = createContext<Prefs | null>(null);

export function PrefsProvider({ children }: { children: React.ReactNode }) {
  const scheme = useColorScheme();
  const system =
    Platform.OS === 'web' ? 'en' : (Localization.getLocales()[0]?.languageCode ?? 'en');
  const [lang, setLangState] = useState<Lang>(detectLang(system));
  useEffect(() => {
    getSecret('orochi.lang').then((saved) => {
      if (saved === 'en' || saved === 'ja') setLangState(saved);
    });
  }, []);
  const value = useMemo<Prefs>(
    () => ({
      lang,
      setLang(next) {
        setLangState(next);
        setSecret('orochi.lang', next);
      },
      dark: scheme === 'dark',
      colors: scheme === 'dark' ? palette.dark : palette.light,
      t: (key) => translate(key, lang),
    }),
    [lang, scheme],
  );
  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}

export function usePrefs(): Prefs {
  const value = useContext(Ctx);
  if (!value) throw new Error('PrefsProvider missing');
  return value;
}
