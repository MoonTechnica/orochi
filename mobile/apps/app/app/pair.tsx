import React, { useEffect, useRef, useState } from 'react';
import {
  ActivityIndicator,
  KeyboardAvoidingView,
  Linking,
  Platform,
  Pressable,
  ScrollView,
  Text,
  TextInput,
  View,
} from 'react-native';
import { CameraView, useCameraPermissions } from 'expo-camera';
import { useLocalSearchParams, useRouter } from 'expo-router';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import { RemoteClient, baseUrl, parsePairingLink } from '@orochi/protocol';
import { usePrefs } from '../src/state/prefs';
import { deviceName, installId, useSession } from '../src/state/session';

export default function Pair() {
  const { colors, t } = usePrefs();
  const { pair, machines, active, switchTo } = useSession();
  const router = useRouter();
  const params = useLocalSearchParams<{
    host?: string;
    port?: string;
    code?: string;
    name?: string;
    id?: string;
    add?: string;
    repair?: string;
    scan?: string;
  }>();
  const repairing = machines.find((m) => m.key === params.repair) ?? null;
  const [done, setDone] = useState<{
    key: string;
    name: string;
    outcome: 'added' | 'updated';
  } | null>(null);
  const [host, setHost] = useState(repairing?.host ?? '');
  const [port, setPort] = useState(String(repairing?.port ?? 47821));
  const [code, setCode] = useState('');
  const linked = useRef<string | null>(null);
  const insets = useSafeAreaInsets();
  const [permission, ask] = useCameraPermissions();
  const [scanning, setScanning] = useState(params.scan === '1');
  const [cameraError, setCameraError] = useState(false);
  const [link, setLink] = useState('');
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);

  const go = async (text: string) => {
    if (busy) return;
    const parsed = parsePairingLink(text);
    if (!parsed) {
      setMessage(t('That is not an Orochi pairing link.'));
      return;
    }
    setBusy(true);
    setMessage(null);
    try {
      const client = new RemoteClient({ base: baseUrl(parsed.host, parsed.port) });
      const result = await client.pair(parsed.code, deviceName(), await installId());
      // An id from the link, else the pairing answer, else /health (a desktop that has none: by address).
      const machineId =
        parsed.id ?? result.id ?? (await client.health().catch(() => null))?.id ?? undefined;
      const known = active?.key;
      const saved = await pair(
        {
          host: parsed.host,
          port: parsed.port,
          token: result.token,
          name: result.name || parsed.name,
          machineId,
        },
        repairing?.key,
      );
      if (known && saved.machine.key !== known)
        setDone({ key: saved.machine.key, name: saved.machine.name, outcome: saved.outcome });
      else router.replace('/');
    } catch (e: any) {
      setMessage(
        e?.status === 401
          ? t('The code expired or was used. Make a new one on the desktop.')
          : `${t('Pairing failed')}: ${e?.message ?? e}`,
      );
      setScanning(false);
    } finally {
      setBusy(false);
    }
  };

  // `orochi://pair?...` opened from a link or `simctl openurl` arrives as route params.
  useEffect(() => {
    if (!params.host || !params.code) return;
    const link = `orochi://pair?host=${encodeURIComponent(params.host)}&port=${params.port ?? ''}&code=${encodeURIComponent(params.code)}&name=${encodeURIComponent(params.name ?? '')}${params.id ? `&id=${encodeURIComponent(params.id)}` : ''}`;
    if (linked.current === link) return;
    linked.current = link;
    go(link);
  }, [params.host, params.port, params.code, params.name, params.id]); // eslint-disable-line react-hooks/exhaustive-deps

  const field = (label: string, value: string, set: (v: string) => void, extra: object = {}) => (
    <View style={{ gap: 4 }}>
      <Text style={{ color: colors.sub, fontSize: 13 }}>{label}</Text>
      <TextInput
        value={value}
        onChangeText={set}
        accessibilityLabel={label}
        placeholderTextColor={colors.sub}
        autoCapitalize="none"
        autoCorrect={false}
        style={{
          minHeight: 48,
          borderRadius: 12,
          borderWidth: 1,
          borderColor: colors.line,
          paddingHorizontal: 12,
          color: colors.text,
          backgroundColor: colors.card,
          fontSize: 16,
        }}
        {...extra}
      />
    </View>
  );

  const button = (label: string, onPress: () => void, primary = true) => (
    <Pressable
      onPress={onPress}
      disabled={busy}
      accessibilityRole="button"
      style={{
        minHeight: 50,
        borderRadius: 12,
        alignItems: 'center',
        justifyContent: 'center',
        backgroundColor: primary ? colors.primary : colors.card,
        borderWidth: primary ? 0 : 1,
        borderColor: colors.line,
        opacity: busy ? 0.6 : 1,
      }}
    >
      <Text
        style={{ color: primary ? colors.onPrimary : colors.text, fontSize: 16, fontWeight: '700' }}
      >
        {label}
      </Text>
    </Pressable>
  );

  return (
    <KeyboardAvoidingView
      style={{ flex: 1, backgroundColor: colors.bg }}
      behavior={Platform.OS === 'ios' ? 'padding' : undefined}
    >
      <ScrollView
        contentContainerStyle={{ padding: 20, paddingTop: insets.top + 24, gap: 16 }}
        keyboardShouldPersistTaps="handled"
      >
        {machines.length > 0 ? (
          <Pressable
            accessibilityRole="button"
            onPress={() => (router.canGoBack() ? router.back() : router.replace('/'))}
            style={{ minHeight: 44, justifyContent: 'center' }}
          >
            <Text style={{ color: colors.text, fontSize: 16 }}>‹ {t('Cancel')}</Text>
          </Pressable>
        ) : null}
        <Text style={{ color: colors.text, fontSize: 30, fontWeight: '800' }}>
          {machines.length > 0 && !repairing ? t('Add a desktop') : t('Pair with your desktop')}
        </Text>
        {done ? (
          <View
            accessibilityLabel="paired"
            style={{
              gap: 10,
              padding: 16,
              borderRadius: 10,
              borderWidth: 1,
              borderColor: colors.line,
              backgroundColor: colors.card,
            }}
          >
            <Text style={{ color: colors.text, fontSize: 17, fontWeight: '700' }}>
              {done.outcome === 'added' ? t('Added') : t('Updated')}: {done.name}
            </Text>
            {button(`${t('Switch to')} ${done.name}`, async () => {
              await switchTo(done.key);
              router.replace('/');
            })}
            {button(t('Stay here'), () => router.replace('/'), false)}
          </View>
        ) : null}
        {repairing ? (
          <Text style={{ color: colors.text, fontSize: 14 }}>
            {t('Pairing again with')} {repairing.label || repairing.name}
          </Text>
        ) : null}
        <Text style={{ color: colors.sub, fontSize: 15 }}>
          {t('Scan the QR code shown in Orochi: Settings → Mobile.')}
        </Text>
        {scanning && permission?.granted && cameraError ? (
          <Text style={{ color: colors.sub }}>
            {t('No camera is available. Paste the link or enter the details below.')}
          </Text>
        ) : null}
        {scanning && permission?.granted && !cameraError ? (
          <View style={{ height: 320, borderRadius: 12, overflow: 'hidden' }}>
            <CameraView
              style={{ flex: 1 }}
              facing="back"
              onMountError={() => setCameraError(true)}
              barcodeScannerSettings={{ barcodeTypes: ['qr'] }}
              onBarcodeScanned={({ data }) => {
                if (parsePairingLink(data)) {
                  setScanning(false);
                  go(data);
                }
              }}
            />
          </View>
        ) : null}
        {scanning && permission?.granted && !cameraError ? (
          <Text style={{ color: colors.sub, fontSize: 13 }}>
            {t('You can also paste the link or enter the details below.')}
          </Text>
        ) : null}
        {scanning && permission && !permission.granted ? (
          <View style={{ gap: 10 }}>
            <Text style={{ color: colors.sub }}>{t('Camera permission is needed to scan.')}</Text>
            {permission.canAskAgain
              ? button(t('Allow camera'), () => ask())
              : button(t('Open Settings'), () => Linking.openSettings())}
            <Text style={{ color: colors.sub, fontSize: 13 }}>
              {t('You can also paste the link or enter the details below.')}
            </Text>
          </View>
        ) : null}
        {button(scanning ? t('Cancel') : t('Scan QR code'), () => {
          setScanning(!scanning);
          setCameraError(false);
          if (!scanning && !permission?.granted) ask();
        })}
        <Text style={{ color: colors.sub, fontSize: 13, fontWeight: '700', marginTop: 8 }}>
          {t('Paste the pairing link')}
        </Text>
        <TextInput
          value={link}
          onChangeText={setLink}
          placeholder="orochi://pair?host=…&port=…&code=…"
          placeholderTextColor={colors.sub}
          autoCapitalize="none"
          autoCorrect={false}
          style={{
            minHeight: 50,
            borderRadius: 12,
            borderWidth: 1,
            borderColor: colors.line,
            paddingHorizontal: 12,
            color: colors.text,
            backgroundColor: colors.card,
            fontSize: 15,
          }}
        />
        {button(t('Pair'), () => go(link), false)}
        <Text style={{ color: colors.sub, fontSize: 13, fontWeight: '700', marginTop: 8 }}>
          {t('Or enter the details')}
        </Text>
        {field(t('Address (Tailscale IP or name)'), host, setHost, { keyboardType: 'url' })}
        {field(t('Port'), port, setPort, { keyboardType: 'number-pad' })}
        {field(t('Pairing code'), code, setCode)}
        {button(
          t('Pair'),
          () =>
            go(
              `orochi://pair?host=${encodeURIComponent(host.trim())}&port=${port.trim()}&code=${encodeURIComponent(code.trim())}`,
            ),
          false,
        )}
        {busy ? <ActivityIndicator color={colors.text} /> : null}
        {message ? <Text style={{ color: colors.dangerInk, fontSize: 14 }}>{message}</Text> : null}
      </ScrollView>
    </KeyboardAvoidingView>
  );
}
