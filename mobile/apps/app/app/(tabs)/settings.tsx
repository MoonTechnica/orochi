import React from 'react';
import { Alert, Platform, Pressable, ScrollView, Switch, Text, View } from 'react-native';
import Constants from 'expo-constants';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import { useData } from '../../src/state/data';
import { Logo } from '../../src/ui/Logo';
import { usePrefs } from '../../src/state/prefs';
import { useSession } from '../../src/state/session';
import { useNotify } from '../../src/state/notify';
import { MachineList } from '../../src/ui/Machines';

export default function Settings() {
  const { paired, remove } = useSession();
  const { colors, t, lang, setLang, dark } = usePrefs();
  const { connection, error, refresh } = useData();
  const insets = useSafeAreaInsets();
  const notify = useNotify();

  const confirmUnpair = () => {
    if (Platform.OS === 'web') {
      if (globalThis.confirm?.(t('Remove this desktop?'))) remove(paired!.key);
      return;
    }
    Alert.alert(t('Remove this desktop?'), paired?.name, [
      { text: t('Cancel'), style: 'cancel' },
      { text: t('Remove'), style: 'destructive', onPress: () => paired && remove(paired.key) },
    ]);
  };

  const row = (label: string, value: string, color?: string) => (
    <View
      style={{
        flexDirection: 'row',
        justifyContent: 'space-between',
        paddingVertical: 12,
        gap: 12,
        minHeight: 48,
      }}
    >
      <Text style={{ color: colors.text, fontSize: 16 }}>{label}</Text>
      <Text
        style={{ color: color ?? colors.sub, fontSize: 16, flexShrink: 1, textAlign: 'right' }}
        numberOfLines={2}
      >
        {value}
      </Text>
    </View>
  );
  const card = (children: React.ReactNode) => (
    <View
      style={{
        marginHorizontal: 16,
        marginTop: 14,
        paddingHorizontal: 16,
        borderRadius: 10,
        backgroundColor: colors.card,
        borderWidth: 1,
        borderColor: colors.line,
      }}
    >
      {children}
    </View>
  );

  return (
    <ScrollView
      keyboardShouldPersistTaps="handled"
      style={{ flex: 1, backgroundColor: colors.bg }}
      contentContainerStyle={{ paddingTop: insets.top + 10, paddingBottom: 40 }}
    >
      <Text
        style={{
          color: colors.text,
          fontSize: 28,
          fontWeight: '800',
          paddingHorizontal: 16,
        }}
      >
        {t('Settings')}
      </Text>
      {card(
        <>
          <Text style={{ color: colors.sub, fontSize: 13, paddingTop: 12 }}>{t('Desktops')}</Text>
          <MachineList />
        </>,
      )}
      {card(
        <>
          {row(t('Machine'), paired?.name ?? '')}
          {row('Address', paired ? `${paired.host}:${paired.port}` : '')}
          {row(
            t('Connection'),
            error
              ? `${t('Error')}: ${error}`
              : connection === 'open'
                ? t('Connected')
                : connection === 'connecting'
                  ? t('Reconnecting')
                  : t('Offline'),
            connection === 'open' ? colors.text : colors.text,
          )}
          <Pressable
            onPress={refresh}
            accessibilityRole="button"
            style={{ minHeight: 48, justifyContent: 'center' }}
          >
            <Text style={{ color: colors.text, fontSize: 16 }}>{t('Retry')}</Text>
          </Pressable>
        </>,
      )}
      {card(
        <>
          <View style={{ flexDirection: 'row', alignItems: 'center', minHeight: 56, gap: 12 }}>
            <Text style={{ color: colors.text, fontSize: 16, flex: 1 }}>{t('Notifications')}</Text>
            <Switch
              accessibilityLabel={t('Notifications')}
              value={notify.enabled}
              onValueChange={(on) => notify.setEnabled(on)}
              disabled={notify.permission === 'denied'}
            />
          </View>
          <Text style={{ color: colors.sub, fontSize: 12, paddingBottom: 12 }}>
            {notify.permission === 'denied'
              ? t('Notifications are blocked in system settings.')
              : t(
                  'Local only, raised while the app is open or just left. Nothing is sent to a push service. A closed app gets none.',
                )}
          </Text>
        </>,
      )}
      {card(
        <>
          <Text style={{ color: colors.sub, fontSize: 13, paddingTop: 12 }}>{t('Language')}</Text>
          <View style={{ flexDirection: 'row', gap: 8, paddingVertical: 10 }}>
            {(['en', 'ja'] as const).map((l) => (
              <Pressable
                key={l}
                onPress={() => setLang(l)}
                accessibilityRole="button"
                accessibilityState={{ selected: lang === l }}
                style={{
                  flex: 1,
                  minHeight: 44,
                  borderRadius: 10,
                  alignItems: 'center',
                  justifyContent: 'center',
                  backgroundColor: lang === l ? colors.text : colors.bg,
                  borderWidth: 1,
                  borderColor: lang === l ? colors.text : colors.line,
                }}
              >
                <Text style={{ color: lang === l ? colors.bg : colors.text, fontWeight: '600' }}>
                  {l === 'en' ? t('English') : t('Japanese')}
                </Text>
              </Pressable>
            ))}
          </View>
          {row(t('Theme'), `${t('Follows the system')} (${dark ? 'dark' : 'light'})`)}
        </>,
      )}
      {card(
        <>
          <View
            style={{ flexDirection: 'row', alignItems: 'center', gap: 12, paddingVertical: 14 }}
          >
            <Logo size={44} />
            <View style={{ flex: 1 }}>
              <Text style={{ color: colors.text, fontWeight: '800', fontSize: 20 }}>Orochi</Text>
              <Text style={{ color: colors.sub, fontSize: 13 }}>{t('Eight heads, one knot.')}</Text>
            </View>
          </View>
          {row(t('Version'), String(Constants.expoConfig?.version ?? '0.1.0'))}
          <Pressable
            onPress={confirmUnpair}
            accessibilityRole="button"
            style={{ minHeight: 52, justifyContent: 'center' }}
          >
            <Text style={{ color: colors.dangerInk, fontSize: 16, fontWeight: '600' }}>
              {t('Remove this desktop')}
            </Text>
          </Pressable>
        </>,
      )}
    </ScrollView>
  );
}
