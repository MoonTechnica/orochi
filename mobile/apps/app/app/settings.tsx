import React, { useState } from 'react';
import { Pressable, ScrollView, Switch, Text, View } from 'react-native';
import Constants from 'expo-constants';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import { usePrefs } from '../src/state/prefs';
import { useNotify } from '../src/state/notify';
import { Disclosure } from '../src/ui/Disclosure';
import { MachineList } from '../src/ui/Machines';
import { Header } from '../src/ui/Screen';

type Section = 'desktops' | 'notifications' | 'language' | 'about';

export default function Settings() {
  const { colors, t, lang, setLang } = usePrefs();
  const insets = useSafeAreaInsets();
  const notify = useNotify();
  const [section, setSection] = useState<Section | null>(null);
  const toggle = (key: Section) => setSection((current) => (current === key ? null : key));
  const version = String(Constants.expoConfig?.version ?? '0.1.0');
  return (
    <View style={{ flex: 1, backgroundColor: colors.bg }}>
      <Header title={t('Settings')} />
      <ScrollView
        keyboardShouldPersistTaps="handled"
        contentContainerStyle={{ padding: 16, paddingBottom: Math.max(24, insets.bottom) }}
      >
        <Disclosure
          title={t('Desktops')}
          icon="desktop-outline"
          testID="settings-desktops"
          expanded={section === 'desktops'}
          onPress={() => toggle('desktops')}
        >
          <View style={{ paddingHorizontal: 12 }}>
            <MachineList />
          </View>
        </Disclosure>
        <Disclosure
          title={t('Notifications')}
          icon="notifications-outline"
          expanded={section === 'notifications'}
          onPress={() => toggle('notifications')}
        >
          <View style={{ paddingHorizontal: 12, paddingBottom: 12, gap: 6 }}>
            <View style={{ flexDirection: 'row', alignItems: 'center', minHeight: 44, gap: 12 }}>
              <Text style={{ color: colors.text, fontSize: 15, flex: 1 }}>
                {t('Notifications')}
              </Text>
              <Switch
                testID="notifications-switch"
                accessibilityLabel={t('Notifications')}
                disabled={notify.permission === 'denied'}
                value={notify.enabled}
                onValueChange={(on) => notify.setEnabled(on)}
              />
            </View>
            <Text style={{ color: colors.sub, fontSize: 13 }}>
              {notify.permission === 'denied'
                ? t('Notifications are blocked in system settings.')
                : t(
                    'Local only, raised while the app is open or just left. Nothing is sent to a push service. A closed app gets none.',
                  )}
            </Text>
          </View>
        </Disclosure>
        <Disclosure
          title={t('Language')}
          summary={lang === 'en' ? t('English') : t('Japanese')}
          icon="language-outline"
          expanded={section === 'language'}
          onPress={() => toggle('language')}
        >
          {(['en', 'ja'] as const).map((value) => (
            <Pressable
              key={value}
              onPress={() => setLang(value)}
              accessibilityRole="button"
              aria-pressed={lang === value}
              style={{ minHeight: 44, justifyContent: 'center', paddingHorizontal: 12 }}
            >
              <Text style={{ color: colors.text, fontWeight: lang === value ? '700' : '400' }}>
                {value === 'en' ? t('English') : t('Japanese')}
              </Text>
            </Pressable>
          ))}
        </Disclosure>
        <Disclosure
          title={t('About')}
          expanded={section === 'about'}
          onPress={() => toggle('about')}
        >
          <View style={{ paddingHorizontal: 12, paddingBottom: 12, gap: 4 }}>
            <Text style={{ color: colors.sub, fontSize: 14 }}>Orochi {version}</Text>
            <Text style={{ color: colors.sub, fontSize: 14 }}>
              {t('Theme')} · {t('Follows the system')}
            </Text>
          </View>
        </Disclosure>
      </ScrollView>
    </View>
  );
}
