import React from 'react';
import { Tabs } from 'expo-router';
import { useAttention } from '../../src/state/attention';
import { usePrefs } from '../../src/state/prefs';
import { useSession } from '../../src/state/session';
import { TabBar } from '../../src/ui/TabBar';

export const unstable_settings = { initialRouteName: 'index' };

export default function OverviewTabs() {
  const { t } = usePrefs();
  const { machines } = useSession();
  const { home } = useAttention();
  return (
    <Tabs
      tabBar={(props) => (machines.length ? <TabBar {...props} /> : null)}
      screenOptions={{ headerShown: false }}
    >
      <Tabs.Screen name="index" options={{ title: t('Home'), tabBarBadge: home.needs.length }} />
      <Tabs.Screen name="all-conversations" options={{ title: t('Chats') }} />
      <Tabs.Screen name="all-activity" options={{ title: t('Activity') }} />
      <Tabs.Screen name="all-assistants" options={{ title: t('Assistant') }} />
    </Tabs>
  );
}
