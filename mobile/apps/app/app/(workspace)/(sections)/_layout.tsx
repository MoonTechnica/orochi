import React from 'react';
import { View } from 'react-native';
import { Tabs, usePathname, useRouter } from 'expo-router';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import { usePrefs } from '../../../src/state/prefs';
import { useSession } from '../../../src/state/session';
import { MachineSwitcher, RepairPanel } from '../../../src/ui/Machines';
import { NewThreadButton } from '../../../src/ui/NewThread';
import { HeaderButton } from '../../../src/ui/Screen';
import { TabBar } from '../../../src/ui/TabBar';

export const unstable_settings = { initialRouteName: 'conversations' };

export default function WorkspaceSections() {
  const { colors, t } = usePrefs();
  const { active } = useSession();
  const insets = useSafeAreaInsets();
  const router = useRouter();
  const chats = usePathname() === '/conversations';
  return (
    <View style={{ flex: 1, backgroundColor: colors.bg }}>
      <View
        style={{
          paddingTop: insets.top,
          paddingHorizontal: 8,
          flexDirection: 'row',
          alignItems: 'center',
          backgroundColor: colors.card,
          borderBottomWidth: 1,
          borderBottomColor: colors.line,
          minHeight: insets.top + 52,
          gap: 8,
        }}
      >
        <HeaderButton
          testID="workspace-home"
          icon="chevron-back"
          label={t('Home')}
          onPress={() => router.dismissTo('/')}
        />
        <MachineSwitcher />
        {chats && !active?.needsRepair ? <NewThreadButton /> : null}
      </View>
      {active?.needsRepair ? (
        <RepairPanel />
      ) : (
        <Tabs tabBar={(props) => <TabBar {...props} />} screenOptions={{ headerShown: false }}>
          <Tabs.Screen name="conversations" options={{ title: t('Chats') }} />
          <Tabs.Screen name="assistant" options={{ title: t('Assistant') }} />
          <Tabs.Screen name="activity" options={{ title: t('Activity') }} />
        </Tabs>
      )}
    </View>
  );
}
