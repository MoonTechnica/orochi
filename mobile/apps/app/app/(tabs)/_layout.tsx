import React, { useState } from 'react';
import { View } from 'react-native';
import { Tabs, usePathname } from 'expo-router';
import { countNeeds } from '@orochi/core';
import { usePrefs } from '../../src/state/prefs';
import { useData } from '../../src/state/data';
import { useSession } from '../../src/state/session';
import { RepairPanel } from '../../src/ui/Machines';
import { TabBar, TabBarHeight } from '../../src/ui/TabBar';

export default function TabsLayout() {
  const { t } = usePrefs();
  const { projects, prompts, mentions } = useData();
  const needs =
    countNeeds(projects) + prompts.length + mentions.length > 0 ? countNeeds(projects) : 0;
  const activity = prompts.length + mentions.length;
  const { active, machines } = useSession();
  const empty = machines.length === 0;
  const [bar, setBar] = useState(0);
  const onSettings = usePathname().endsWith('/settings');
  // A desktop that needs pairing again covers the screens but never the bar: the other
  // desktops, Settings and removing this one all stay reachable.
  return (
    <TabBarHeight.Provider value={{ height: bar, set: setBar }}>
      <View style={{ flex: 1 }}>
        <Tabs
          tabBar={(props) => <TabBar {...(props as any)} />}
          screenOptions={{ headerShown: false }}
        >
          <Tabs.Screen
            name="index"
            options={{
              title: t('Home'),
              tabBarBadge: activity + needs > 0 ? activity + needs : undefined,
            }}
          />
          <Tabs.Screen
            name="assistant"
            options={{
              title: t('Assistant'),
              href: empty ? null : undefined,
            }}
          />
          <Tabs.Screen
            name="conversations"
            options={{
              title: t('Conversations'),
              href: empty ? null : undefined,
              tabBarBadge: needs || undefined,
            }}
          />
          <Tabs.Screen
            name="activity"
            options={{
              title: t('Activity'),
              href: empty ? null : undefined,
              tabBarBadge: activity || undefined,
            }}
          />
          <Tabs.Screen
            name="settings"
            options={{
              title: t('Settings'),
            }}
          />
        </Tabs>
        {active?.needsRepair && !onSettings ? (
          <View
            testID="repair-cover"
            style={{ position: 'absolute', top: 0, left: 0, right: 0, bottom: bar }}
          >
            <RepairPanel />
          </View>
        ) : null}
      </View>
    </TabBarHeight.Provider>
  );
}
