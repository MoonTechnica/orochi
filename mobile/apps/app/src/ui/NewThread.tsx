import React, { useState } from 'react';
import { Modal, Pressable, ScrollView, Text, TextInput, View } from 'react-native';
import { RemoteClient, api, baseUrl } from '@orochi/protocol';
import type { Browse } from '@orochi/protocol';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import { usePrefs } from '../state/prefs';
import { useAttention } from '../state/attention';
import { useOpenThread } from '../state/open';
import { useSession } from '../state/session';
import { Icon } from './Icon';
import { radius } from './theme';

export function NewThreadButton() {
  const { colors, t } = usePrefs();
  const [visible, setVisible] = useState(false);
  return (
    <>
      <Pressable
        testID="new-thread"
        accessibilityRole="button"
        accessibilityLabel={t('New conversation')}
        onPress={() => setVisible(true)}
        style={({ pressed }) => ({
          width: 44,
          height: 44,
          borderRadius: radius.tile,
          backgroundColor: colors.primary,
          alignItems: 'center',
          justifyContent: 'center',
          opacity: pressed ? 0.75 : 1,
        })}
      >
        <Icon name="add" size={24} color={colors.onPrimary} />
      </Pressable>
      <NewThreadSheet visible={visible} onClose={() => setVisible(false)} />
    </>
  );
}

type View_ =
  | { kind: 'list' }
  | { kind: 'folder'; machineKey: string; listing: Browse | null }
  | { kind: 'clone'; machineKey: string };

/**
 * Pick a project in the active workspace, then `new_thread` and open it. A project the
 * desktop does not have yet is added from one of its folders or cloned from an address.
 */
export function NewThreadSheet({ visible, onClose }: { visible: boolean; onClose: () => void }) {
  const { colors, t } = usePrefs();
  const { active } = useSession();
  const { snapshots } = useAttention();
  const open = useOpenThread();
  const insets = useSafeAreaInsets();
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const online = snapshots.filter((s) => s.key === active?.key && s.state === 'online');
  const [view, setView] = useState<View_>({ kind: 'list' });
  const [address, setAddress] = useState('');

  const clientOf = (machineKey: string) => {
    const m = active?.key === machineKey ? active : null;
    return m ? api(new RemoteClient({ base: baseUrl(m.host, m.port), token: m.token })) : null;
  };
  const close = () => {
    setView({ kind: 'list' });
    setError(null);
    onClose();
  };
  const browse = async (machineKey: string, path: string | null) => {
    const c = clientOf(machineKey);
    if (!c) return;
    setBusy('browse');
    setError(null);
    try {
      setView({ kind: 'folder', machineKey, listing: await c.browse(path) });
    } catch (e: any) {
      setError(String(e?.message ?? e));
      setView((v) => (v.kind === 'folder' ? v : { kind: 'folder', machineKey, listing: null }));
    } finally {
      setBusy(null);
    }
  };
  const adopt = async (
    machineKey: string,
    run: (c: NonNullable<ReturnType<typeof clientOf>>) => Promise<{ root: string }>,
  ) => {
    const c = clientOf(machineKey);
    if (!c) return;
    setBusy('adopt');
    setError(null);
    try {
      const { root } = await run(c);
      const id = await c.newThread(root);
      close();
      await open(machineKey, id);
    } catch (e: any) {
      setError(String(e?.message ?? e));
    } finally {
      setBusy(null);
    }
  };

  const start = async (machineKey: string, root: string) => {
    const m = active?.key === machineKey ? active : null;
    if (!m) return;
    setBusy(`${machineKey}:${root}`);
    setError(null);
    try {
      const id = await api(
        new RemoteClient({ base: baseUrl(m.host, m.port), token: m.token }),
      ).newThread(root);
      close();
      await open(machineKey, id);
    } catch (e: any) {
      setError(String(e?.message ?? e));
    } finally {
      setBusy(null);
    }
  };

  return (
    <Modal visible={visible} transparent animationType="fade" onRequestClose={close}>
      <View
        accessibilityViewIsModal
        style={{ flex: 1, backgroundColor: '#0006', justifyContent: 'flex-end' }}
      >
        <Pressable
          style={{ position: 'absolute', top: 0, bottom: 0, left: 0, right: 0 }}
          onPress={close}
          accessibilityRole="button"
          accessibilityLabel={t('Close menu')}
        />
        <View
          style={{
            backgroundColor: colors.card,
            borderTopLeftRadius: 12,
            borderTopRightRadius: 12,
            paddingBottom: insets.bottom + 8,
            maxHeight: '75%',
          }}
        >
          <Text
            style={{
              color: colors.text,
              fontSize: 18,
              fontWeight: '800',
              padding: 16,
              paddingBottom: 6,
            }}
          >
            {t('New conversation')}
          </Text>
          {view.kind === 'folder' ? (
            <FolderView
              listing={view.listing}
              busy={!!busy}
              error={error}
              onOpen={(path) => browse(view.machineKey, path)}
              onBack={() => {
                setError(null);
                setView({ kind: 'list' });
              }}
              onUse={(root) => adopt(view.machineKey, (c) => c.addProject(root))}
            />
          ) : view.kind === 'clone' ? (
            <ScrollView keyboardShouldPersistTaps="handled">
              <Action
                label={t('Back')}
                disabled={!!busy}
                onPress={() => {
                  setError(null);
                  setView({ kind: 'list' });
                }}
              />
              <Text style={{ color: colors.sub, fontSize: 12, padding: 16, paddingBottom: 6 }}>
                {t('Repository address')}
              </Text>
              <TextInput
                value={address}
                onChangeText={setAddress}
                autoCapitalize="none"
                autoCorrect={false}
                keyboardType="url"
                placeholder="https://github.com/owner/repo.git"
                placeholderTextColor={colors.sub}
                accessibilityLabel={t('Repository address')}
                editable={!busy}
                style={{
                  color: colors.text,
                  marginHorizontal: 16,
                  minHeight: 44,
                  borderWidth: 1,
                  borderColor: colors.line,
                  borderRadius: 8,
                  paddingHorizontal: 12,
                }}
              />
              <Action
                label={busy === 'adopt' ? t('Cloning…') : t('Clone')}
                disabled={!!busy || address.trim() === ''}
                onPress={() =>
                  adopt(view.machineKey, async (c) => {
                    const made = await c.cloneProject(address.trim());
                    setAddress('');
                    return made;
                  })
                }
              />
              {error ? <Text style={{ color: colors.dangerInk, padding: 16 }}>{error}</Text> : null}
            </ScrollView>
          ) : (
            <ScrollView>
              {online.length === 0 ? (
                <Text style={{ color: colors.sub, padding: 16 }}>
                  {t('This desktop does not answer.')}
                </Text>
              ) : null}
              {online.map((s) => (
                <View key={s.key}>
                  {s.projects
                    .filter((p) => !p.missing)
                    .map((p) => (
                      <Pressable
                        key={p.id}
                        accessibilityRole="button"
                        accessibilityLabel={p.name}
                        disabled={!!busy}
                        onPress={() => start(s.key, p.root)}
                        style={{
                          minHeight: 52,
                          justifyContent: 'center',
                          paddingHorizontal: 16,
                          borderTopWidth: 1,
                          borderTopColor: colors.line,
                          opacity: busy ? 0.6 : 1,
                        }}
                      >
                        <Text style={{ color: colors.text, fontSize: 16 }}>{p.name}</Text>
                        <Text style={{ color: colors.sub, fontSize: 12 }} numberOfLines={1}>
                          {p.root}
                        </Text>
                      </Pressable>
                    ))}
                  <Action
                    label={t('Add a folder…')}
                    disabled={!!busy}
                    onPress={() => browse(s.key, null)}
                  />
                  <Action
                    label={t('Clone a repository…')}
                    disabled={!!busy}
                    onPress={() => {
                      setError(null);
                      setView({ kind: 'clone', machineKey: s.key });
                    }}
                  />
                </View>
              ))}
              {error ? <Text style={{ color: colors.dangerInk, padding: 16 }}>{error}</Text> : null}
            </ScrollView>
          )}
        </View>
      </View>
    </Modal>
  );
}

function Action({
  label,
  onPress,
  disabled,
}: {
  label: string;
  onPress: () => void;
  disabled?: boolean;
}) {
  const { colors } = usePrefs();
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={label}
      disabled={disabled}
      onPress={onPress}
      style={{
        minHeight: 52,
        justifyContent: 'center',
        paddingHorizontal: 16,
        borderTopWidth: 1,
        borderTopColor: colors.line,
        opacity: disabled ? 0.6 : 1,
      }}
    >
      <Text style={{ color: colors.text, fontSize: 16, fontWeight: '600' }}>{label}</Text>
    </Pressable>
  );
}

/** The folders of one desktop, one level at a time, to pick the one that becomes a project. */
function FolderView({
  listing,
  busy,
  error,
  onOpen,
  onBack,
  onUse,
}: {
  listing: Browse | null;
  busy: boolean;
  error: string | null;
  onOpen: (path: string) => void;
  onBack: () => void;
  onUse: (path: string) => void;
}) {
  const { colors, t } = usePrefs();
  return (
    <ScrollView>
      <Action label={t('Back')} disabled={busy} onPress={onBack} />
      {listing ? (
        <>
          <Text
            style={{ color: colors.sub, fontSize: 12, padding: 16, paddingBottom: 6 }}
            numberOfLines={2}
          >
            {listing.path}
          </Text>
          <Action
            label={t('Use this folder')}
            disabled={busy}
            onPress={() => onUse(listing.path)}
          />
          {listing.parent ? (
            <Action label={t('Up')} disabled={busy} onPress={() => onOpen(listing.parent!)} />
          ) : null}
          {listing.entries.length === 0 ? (
            <Text style={{ color: colors.sub, padding: 16 }}>{t('No folders here.')}</Text>
          ) : null}
          {listing.entries.map((e) => (
            <Pressable
              key={e.path}
              accessibilityRole="button"
              accessibilityLabel={e.name}
              disabled={busy}
              onPress={() => onOpen(e.path)}
              style={{
                minHeight: 48,
                justifyContent: 'center',
                paddingHorizontal: 16,
                borderTopWidth: 1,
                borderTopColor: colors.line,
              }}
            >
              <Text style={{ color: colors.text, fontSize: 16 }}>
                {e.name}
                {e.git ? '  ·  git' : ''}
              </Text>
            </Pressable>
          ))}
        </>
      ) : null}
      {error ? <Text style={{ color: colors.dangerInk, padding: 16 }}>{error}</Text> : null}
    </ScrollView>
  );
}
