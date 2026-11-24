import React from 'react';
import { Redirect, Stack } from 'expo-router';
import { usePrefs } from '../../src/state/prefs';
import { useSession } from '../../src/state/session';

export const unstable_settings = { initialRouteName: '(sections)' };

export default function WorkspaceStack() {
  const { active, ready } = useSession();
  const { colors } = usePrefs();
  if (!ready) return null;
  if (!active) return <Redirect href="/" />;
  return (
    <Stack screenOptions={{ headerShown: false, contentStyle: { backgroundColor: colors.bg } }} />
  );
}
