import React, { useEffect } from 'react';
import { LogBox, View } from 'react-native';
import { Stack, useGlobalSearchParams, useRouter, useSegments } from 'expo-router';
import { StatusBar } from 'expo-status-bar';
import * as SplashScreen from 'expo-splash-screen';
import { SafeAreaProvider } from 'react-native-safe-area-context';
import { PrefsProvider, usePrefs } from '../src/state/prefs';
import { SessionProvider, useSession } from '../src/state/session';
import { AttentionProvider } from '../src/state/attention';
import { DataProvider } from '../src/state/data';
import { NotifyProvider } from '../src/state/notify';
import { PushProvider } from '../src/state/push';

// A native-driven Animated value that has no listener yet; React Native warns and it is harmless.
LogBox.ignoreLogs(['Sending `onAnimatedValueUpdate` with no listeners registered.']);

export const unstable_settings = { initialRouteName: '(overview)' };

SplashScreen.preventAutoHideAsync().catch(() => {});

function Gate({ children }: { children: React.ReactNode }) {
  const { ready, paired, machines } = useSession();
  const segments = useSegments();
  const router = useRouter();
  const params = useGlobalSearchParams<{ code?: string; add?: string; repair?: string }>();
  const linking = !!params.code || !!params.add || !!params.repair;
  useEffect(() => {
    if (!ready) return;
    const onPair = segments[0] === 'pair';
    // With no desktop paired Home is the guide; a pairing link opened while paired is kept.
    if (paired && onPair && !linking && machines.length > 0) router.replace('/');
  }, [ready, paired, segments, router, linking]);
  return <>{children}</>;
}

function Shell() {
  const { colors, dark } = usePrefs();
  return (
    <View style={{ flex: 1, backgroundColor: colors.bg }}>
      <StatusBar style={dark ? 'light' : 'dark'} />
      <Stack screenOptions={{ headerShown: false, contentStyle: { backgroundColor: colors.bg } }}>
        <Stack.Screen
          name="viewer/media"
          options={{ presentation: 'fullScreenModal', animation: 'fade' }}
        />
      </Stack>
    </View>
  );
}

/** Everything below is rebuilt when the active machine changes: nothing it cached carries over. */
function Scoped({ children }: { children: React.ReactNode }) {
  const { active } = useSession();
  return (
    <DataProvider key={active?.key ?? 'none'}>
      <AttentionProvider>{children}</AttentionProvider>
    </DataProvider>
  );
}

export default function Root() {
  useEffect(() => {
    SplashScreen.hideAsync().catch(() => {});
  }, []);
  return (
    <SafeAreaProvider>
      <PrefsProvider>
        <SessionProvider>
          <Scoped>
            <NotifyProvider>
              <PushProvider>
                <Gate>
                  <Shell />
                </Gate>
              </PushProvider>
            </NotifyProvider>
          </Scoped>
        </SessionProvider>
      </PrefsProvider>
    </SafeAreaProvider>
  );
}
