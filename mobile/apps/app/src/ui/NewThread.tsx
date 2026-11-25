import React, { useEffect, useMemo, useState } from 'react';
import {
  ActivityIndicator,
  KeyboardAvoidingView,
  Modal,
  Platform,
  Pressable,
  ScrollView,
  Text,
  TextInput,
  View,
} from 'react-native';
import { RemoteClient, api, baseUrl } from '@orochi/protocol';
import type { Browse, Placement } from '@orochi/protocol';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import { usePrefs } from '../state/prefs';
import { useAttention } from '../state/attention';
import { useOpenThread } from '../state/open';
import { useSession } from '../state/session';
import { Icon, type IconName } from './Icon';

export const FAB = 56;

/** A floating action button over the bottom-right of the list it starts a conversation in. */
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
          position: 'absolute',
          right: 18,
          bottom: 18,
          width: FAB,
          height: FAB,
          borderRadius: FAB / 2,
          backgroundColor: colors.primary,
          alignItems: 'center',
          justifyContent: 'center',
          opacity: pressed ? 0.75 : 1,
          shadowColor: '#000',
          shadowOpacity: 0.2,
          shadowRadius: 8,
          shadowOffset: { width: 0, height: 3 },
          elevation: 6,
        })}
      >
        <Icon name="add" size={28} color={colors.onPrimary} />
      </Pressable>
      <NewThreadSheet visible={visible} onClose={() => setVisible(false)} />
    </>
  );
}

type Page =
  | { kind: 'form' }
  | { kind: 'projects' }
  | { kind: 'place' }
  | { kind: 'folder'; listing: Browse | null }
  | { kind: 'clone' };

interface Chosen {
  name: string;
  root: string;
}

/**
 * A form, as the desktop's own new-thread dialog is: which project, on which machine, and
 * whether in the project's folder or a worktree of its own; then `new_thread` and open it. A
 * project the desktop does not have yet is added from a folder or cloned, and then chosen here
 * rather than started at once, so the worktree choice still applies to it.
 */
export function NewThreadSheet({ visible, onClose }: { visible: boolean; onClose: () => void }) {
  const { colors, t } = usePrefs();
  const { active } = useSession();
  const { snapshots } = useAttention();
  const open = useOpenThread();
  const insets = useSafeAreaInsets();
  const [busy, setBusy] = useState<'start' | 'browse' | 'adopt' | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [page, setPage] = useState<Page>({ kind: 'form' });
  const [address, setAddress] = useState('');
  const [chosen, setChosen] = useState<Chosen | null>(null);
  const [worktree, setWorktree] = useState(false);
  // Where it runs: read per project, so only the places that project already has are offered.
  const [placement, setPlacement] = useState<Placement | null>(null);
  const [place, setPlace] = useState<string | null>(null);

  const machine = snapshots.find((s) => s.key === active?.key && s.state === 'online') ?? null;
  const projects = useMemo(
    () => (machine?.projects ?? []).filter((p) => !p.missing),
    [machine?.projects],
  );
  // The project last worked in comes first, as it is the one most often meant.
  const recent = useMemo(() => {
    const last = (p: (typeof projects)[number]) =>
      Math.max(0, ...p.threads.map((th) => th.updated_at ?? 0));
    return [...projects].sort((a, b) => last(b) - last(a))[0] ?? null;
  }, [projects]);
  useEffect(() => {
    if (visible && !chosen && recent) setChosen({ name: recent.name, root: recent.root });
  }, [visible, chosen, recent]);

  useEffect(() => {
    setPlacement(null);
    setPlace(null);
    const c = visible && chosen ? client() : null;
    if (!c || !chosen) return;
    let live = true;
    c.placement(chosen.root)
      .then((p) => {
        if (!live) return;
        setPlacement(p);
        setPlace(p.ready.includes(p.mode) ? p.mode : 'host');
      })
      // A desktop that predates the read keeps the project's own setting.
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [visible, chosen?.root, active?.key]);

  const client = () =>
    active
      ? api(new RemoteClient({ base: baseUrl(active.host, active.port), token: active.token }))
      : null;
  const close = () => {
    if (busy === 'start') return;
    setPage({ kind: 'form' });
    setError(null);
    setWorktree(false);
    setChosen(null);
    onClose();
  };
  const run = async (kind: 'start' | 'browse' | 'adopt', job: () => Promise<void>) => {
    setBusy(kind);
    setError(null);
    try {
      await job();
    } catch (e: any) {
      setError(String(e?.message ?? e));
    } finally {
      setBusy(null);
    }
  };
  const browse = (path: string | null) =>
    run('browse', async () => {
      const c = client();
      if (!c) return;
      setPage({ kind: 'folder', listing: await c.browse(path) });
    });
  const adopt = (
    job: (c: NonNullable<ReturnType<typeof client>>) => Promise<{ root: string; name?: string }>,
  ) =>
    run('adopt', async () => {
      const c = client();
      if (!c) return;
      const made = await job(c);
      setChosen({ name: made.name || baseName(made.root), root: made.root });
      setAddress('');
      setPage({ kind: 'form' });
    });
  const start = () =>
    run('start', async () => {
      const c = client();
      if (!c || !chosen || !active) return;
      // Only a choice other than the project's own is sent, as the window does: otherwise the
      // conversation follows the project's setting if it changes later.
      const sandbox = placement && place && place !== placement.mode ? place : null;
      const id = await c.newThread(chosen.root, worktree, sandbox);
      const key = active.key;
      setBusy(null);
      close();
      await open(key, id);
    });

  const back = () => {
    setError(null);
    setPage(
      page.kind === 'folder' || page.kind === 'clone' ? { kind: 'projects' } : { kind: 'form' },
    );
  };
  const title =
    page.kind === 'form'
      ? t('New conversation')
      : page.kind === 'projects'
        ? t('Project')
        : page.kind === 'place'
          ? t('Environment')
          : page.kind === 'folder'
            ? t('Add a folder')
            : t('Clone a repository');

  return (
    <Modal visible={visible} transparent animationType="slide" onRequestClose={close}>
      <KeyboardAvoidingView
        behavior={Platform.OS === 'ios' ? 'padding' : undefined}
        style={{ flex: 1, backgroundColor: '#0006', justifyContent: 'flex-end' }}
      >
        <Pressable
          style={{ position: 'absolute', top: 0, bottom: 0, left: 0, right: 0 }}
          onPress={close}
          accessibilityRole="button"
          accessibilityLabel={t('Close menu')}
        />
        <View
          accessibilityViewIsModal
          testID="new-thread-sheet"
          style={{
            backgroundColor: colors.bg,
            borderTopLeftRadius: 20,
            borderTopRightRadius: 20,
            paddingBottom: insets.bottom + 12,
            maxHeight: '90%',
          }}
        >
          <View
            style={{
              alignSelf: 'center',
              width: 36,
              height: 5,
              borderRadius: 3,
              backgroundColor: colors.line,
              marginTop: 8,
            }}
          />
          <View
            style={{
              flexDirection: 'row',
              alignItems: 'center',
              paddingHorizontal: 8,
              minHeight: 52,
            }}
          >
            <Pressable
              accessibilityRole="button"
              accessibilityLabel={page.kind === 'form' ? t('Cancel') : t('Back')}
              disabled={busy === 'start'}
              onPress={page.kind === 'form' ? close : back}
              hitSlop={6}
              style={{
                minWidth: 88,
                minHeight: 44,
                paddingHorizontal: 8,
                flexDirection: 'row',
                alignItems: 'center',
              }}
            >
              {page.kind === 'form' ? (
                <Text style={{ color: colors.text, fontSize: 16 }}>{t('Cancel')}</Text>
              ) : (
                <>
                  <Icon name="chevron-back" size={22} color={colors.text} />
                  <Text style={{ color: colors.text, fontSize: 16 }}>{t('Back')}</Text>
                </>
              )}
            </Pressable>
            <Text
              accessibilityRole="header"
              numberOfLines={1}
              style={{
                flex: 1,
                textAlign: 'center',
                color: colors.text,
                fontSize: 17,
                fontWeight: '700',
              }}
            >
              {title}
            </Text>
            <View style={{ minWidth: 88 }} />
          </View>

          {!machine ? (
            <Text style={{ color: colors.sub, padding: 20 }}>
              {t('This desktop does not answer.')}
            </Text>
          ) : page.kind === 'form' ? (
            <ScrollView contentContainerStyle={{ paddingHorizontal: 16, paddingBottom: 8 }}>
              <Label text={t('Where')} />
              <Group>
                <Row
                  testID="new-thread-project"
                  icon="folder-outline"
                  label={t('Project')}
                  value={chosen?.name ?? t('Choose a project')}
                  muted={!chosen}
                  onPress={() => setPage({ kind: 'projects' })}
                />
                <Row
                  testID="new-thread-place"
                  icon={place && place !== 'host' ? 'cube-outline' : 'desktop-outline'}
                  label={t('Environment')}
                  value={placeLabel(place ?? 'host', t)}
                  onPress={placement ? () => setPage({ kind: 'place' }) : undefined}
                  divided
                />
              </Group>
              <Label text={t('Work in')} />
              <Group>
                <Choice
                  testID="new-thread-in-place"
                  icon="folder-open-outline"
                  label={t('The project folder')}
                  detail={t('Works on the checkout as it is.')}
                  selected={!worktree}
                  onPress={() => setWorktree(false)}
                />
                <Choice
                  testID="new-thread-worktree"
                  icon="git-branch-outline"
                  label={t('A new worktree')}
                  detail={t('A branch of its own, removed with the conversation.')}
                  selected={worktree}
                  onPress={() => setWorktree(true)}
                  divided
                />
              </Group>
              {error ? <ErrorText text={error} /> : null}
              <PrimaryButton
                testID="new-thread-start"
                label={busy === 'start' ? t('Starting…') : t('Start conversation')}
                busy={busy === 'start'}
                disabled={!chosen || !!busy}
                onPress={start}
              />
            </ScrollView>
          ) : page.kind === 'projects' ? (
            <ScrollView contentContainerStyle={{ paddingHorizontal: 16, paddingBottom: 8 }}>
              {projects.length > 0 ? (
                <Group>
                  {projects.map((p, i) => (
                    <Choice
                      key={p.id}
                      icon="folder-outline"
                      label={p.name}
                      detail={p.root}
                      selected={chosen?.root === p.root}
                      divided={i > 0}
                      onPress={() => {
                        setChosen({ name: p.name, root: p.root });
                        setPage({ kind: 'form' });
                      }}
                    />
                  ))}
                </Group>
              ) : null}
              <Label text={t('Add a project')} />
              <Group>
                <Row
                  icon="add-circle-outline"
                  label={t('Add a folder…')}
                  disabled={!!busy}
                  onPress={() => browse(null)}
                />
                <Row
                  icon="cloud-download-outline"
                  label={t('Clone a repository…')}
                  disabled={!!busy}
                  divided
                  onPress={() => {
                    setError(null);
                    setPage({ kind: 'clone' });
                  }}
                />
              </Group>
              {error ? <ErrorText text={error} /> : null}
            </ScrollView>
          ) : page.kind === 'place' && placement ? (
            <ScrollView contentContainerStyle={{ paddingHorizontal: 16, paddingBottom: 8 }}>
              <Group>
                {placement.options.map((key, i) => {
                  const ready = placement.ready.includes(key);
                  return (
                    <Choice
                      key={key}
                      testID={`new-thread-place-${key}`}
                      icon={key === 'host' ? 'desktop-outline' : 'cube-outline'}
                      label={placeLabel(key, t)}
                      detail={ready ? placeDetail(key, t) : t('Create it on the desktop first')}
                      selected={place === key}
                      disabled={!ready}
                      divided={i > 0}
                      onPress={() => {
                        setPlace(key);
                        setPage({ kind: 'form' });
                      }}
                    />
                  );
                })}
              </Group>
            </ScrollView>
          ) : page.kind === 'folder' ? (
            <FolderPage
              listing={page.listing}
              busy={!!busy}
              error={error}
              onOpen={browse}
              onUse={(root) => adopt((c) => c.addProject(root))}
            />
          ) : (
            <ScrollView
              keyboardShouldPersistTaps="handled"
              contentContainerStyle={{ paddingHorizontal: 16, paddingBottom: 8 }}
            >
              <Label text={t('Repository address')} />
              <TextInput
                value={address}
                onChangeText={setAddress}
                autoCapitalize="none"
                autoCorrect={false}
                autoFocus
                keyboardType="url"
                placeholder="https://github.com/owner/repo.git"
                placeholderTextColor={colors.sub}
                accessibilityLabel={t('Repository address')}
                editable={!busy}
                style={{
                  color: colors.text,
                  backgroundColor: colors.card,
                  minHeight: 52,
                  borderWidth: 1,
                  borderColor: colors.line,
                  borderRadius: 14,
                  paddingHorizontal: 14,
                  fontSize: 16,
                }}
              />
              {error ? <ErrorText text={error} /> : null}
              <PrimaryButton
                label={busy === 'adopt' ? t('Cloning…') : t('Clone')}
                busy={busy === 'adopt'}
                disabled={!!busy || address.trim() === ''}
                onPress={() => adopt((c) => c.cloneProject(address.trim()))}
              />
            </ScrollView>
          )}
        </View>
      </KeyboardAvoidingView>
    </Modal>
  );
}

// Everything runs on the desktop either way, so a place is named by how it is isolated, in a
// word or two, never by the machine's name.
function placeLabel(key: string, t: (k: string) => string) {
  switch (key) {
    case 'host':
      return t('Directly');
    case 'runner':
      return t('Sandbox');
    case 'container':
      return t('Container');
    case 'vm':
      return 'VM';
    default:
      return key;
  }
}

function placeDetail(key: string, t: (k: string) => string) {
  switch (key) {
    case 'host':
      return t('No isolation');
    case 'runner':
      return t('Only tests and servers are isolated');
    case 'container':
      return t('Linux with Docker');
    case 'vm':
      return t('The strongest isolation');
    default:
      return undefined;
  }
}

const baseName = (path: string) =>
  path
    .replace(/[\\/]+$/, '')
    .split(/[\\/]/)
    .pop() || path;

function Label({ text }: { text: string }) {
  const { colors } = usePrefs();
  return (
    <Text
      style={{
        color: colors.sub,
        fontSize: 13,
        fontWeight: '600',
        marginTop: 16,
        marginBottom: 8,
        marginHorizontal: 4,
      }}
    >
      {text}
    </Text>
  );
}

function Group({ children }: { children: React.ReactNode }) {
  const { colors } = usePrefs();
  return (
    <View
      style={{
        backgroundColor: colors.card,
        borderRadius: 14,
        borderWidth: 1,
        borderColor: colors.line,
        overflow: 'hidden',
      }}
    >
      {children}
    </View>
  );
}

/** A divider that starts under the text, past the icon, as grouped lists draw it. */
function Divider() {
  const { colors } = usePrefs();
  return <View style={{ height: 1, marginLeft: 50, backgroundColor: colors.line }} />;
}

function Row({
  icon,
  label,
  value,
  muted,
  divided,
  disabled,
  onPress,
  testID,
}: {
  icon: IconName;
  label: string;
  value?: string;
  muted?: boolean;
  divided?: boolean;
  disabled?: boolean;
  onPress?: () => void;
  testID?: string;
}) {
  const { colors } = usePrefs();
  return (
    <>
      {divided ? <Divider /> : null}
      <Pressable
        testID={testID}
        accessibilityRole={onPress ? 'button' : 'text'}
        accessibilityLabel={[label, value].filter(Boolean).join(', ')}
        disabled={disabled || !onPress}
        onPress={onPress}
        style={({ pressed }) => ({
          minHeight: 54,
          flexDirection: 'row',
          alignItems: 'center',
          gap: 12,
          paddingLeft: 16,
          paddingRight: 12,
          backgroundColor: pressed ? colors.raised : 'transparent',
          opacity: disabled ? 0.5 : 1,
        })}
      >
        <Icon name={icon} size={22} color={colors.text} />
        <Text numberOfLines={2} style={{ color: colors.text, fontSize: 16, flexShrink: 1 }}>
          {label}
        </Text>
        <Text
          numberOfLines={1}
          style={{
            flex: 1,
            minWidth: 0,
            textAlign: 'right',
            color: colors.sub,
            fontSize: 16,
            fontWeight: muted ? '400' : '500',
          }}
        >
          {value ?? ''}
        </Text>
        {onPress ? (
          <Icon name="chevron-forward" size={18} color={colors.sub} />
        ) : (
          <View style={{ width: 18 }} />
        )}
      </Pressable>
    </>
  );
}

function Choice({
  icon,
  label,
  detail,
  selected,
  divided,
  disabled,
  onPress,
  testID,
}: {
  icon: IconName;
  label: string;
  detail?: string;
  selected: boolean;
  divided?: boolean;
  disabled?: boolean;
  onPress: () => void;
  testID?: string;
}) {
  const { colors } = usePrefs();
  return (
    <>
      {divided ? <Divider /> : null}
      <Pressable
        testID={testID}
        accessibilityRole="radio"
        aria-checked={selected}
        aria-disabled={disabled}
        accessibilityLabel={[label, detail].filter(Boolean).join(', ')}
        disabled={disabled}
        onPress={onPress}
        style={({ pressed }) => ({
          minHeight: 60,
          flexDirection: 'row',
          alignItems: 'center',
          gap: 12,
          paddingLeft: 16,
          paddingRight: 14,
          paddingVertical: 10,
          backgroundColor: pressed ? colors.raised : 'transparent',
          opacity: disabled ? 0.45 : 1,
        })}
      >
        <Icon name={icon} size={22} color={colors.text} />
        <View style={{ flex: 1, minWidth: 0, gap: 2 }}>
          <Text
            numberOfLines={2}
            style={{ color: colors.text, fontSize: 16, fontWeight: selected ? '600' : '400' }}
          >
            {label}
          </Text>
          {detail ? (
            <Text numberOfLines={2} style={{ color: colors.sub, fontSize: 13 }}>
              {detail}
            </Text>
          ) : null}
        </View>
        <Icon
          name={selected ? 'checkmark-circle' : 'ellipse-outline'}
          size={22}
          color={selected ? colors.text : colors.line}
        />
      </Pressable>
    </>
  );
}

function PrimaryButton({
  label,
  onPress,
  disabled,
  busy,
  testID,
}: {
  label: string;
  onPress: () => void;
  disabled?: boolean;
  busy?: boolean;
  testID?: string;
}) {
  const { colors } = usePrefs();
  return (
    <Pressable
      testID={testID}
      accessibilityRole="button"
      accessibilityLabel={label}
      aria-disabled={disabled}
      disabled={disabled}
      onPress={onPress}
      style={({ pressed }) => ({
        marginTop: 24,
        minHeight: 56,
        borderRadius: 16,
        backgroundColor: colors.primary,
        flexDirection: 'row',
        alignItems: 'center',
        justifyContent: 'center',
        gap: 10,
        opacity: disabled && !busy ? 0.35 : pressed ? 0.8 : 1,
      })}
    >
      {busy ? <ActivityIndicator color={colors.onPrimary} /> : null}
      <Text style={{ color: colors.onPrimary, fontSize: 17, fontWeight: '700' }}>{label}</Text>
    </Pressable>
  );
}

function ErrorText({ text }: { text: string }) {
  const { colors } = usePrefs();
  return (
    <Text style={{ color: colors.dangerInk, fontSize: 14, marginTop: 12, marginHorizontal: 4 }}>
      {text}
    </Text>
  );
}

/** The folders of one desktop, one level at a time, to pick the one that becomes a project. */
function FolderPage({
  listing,
  busy,
  error,
  onOpen,
  onUse,
}: {
  listing: Browse | null;
  busy: boolean;
  error: string | null;
  onOpen: (path: string) => void;
  onUse: (path: string) => void;
}) {
  const { colors, t } = usePrefs();
  return (
    <View style={{ flexShrink: 1, paddingHorizontal: 16 }}>
      {listing ? (
        <Text
          numberOfLines={2}
          style={{ color: colors.sub, fontSize: 13, marginHorizontal: 4, marginBottom: 8 }}
        >
          {listing.path}
        </Text>
      ) : null}
      <ScrollView style={{ flexShrink: 1 }}>
        {listing ? (
          <Group>
            {listing.parent ? (
              <Row
                icon="arrow-up-outline"
                label={t('Up')}
                disabled={busy}
                onPress={() => onOpen(listing.parent!)}
              />
            ) : null}
            {listing.entries.map((e, i) => (
              <Row
                key={e.path}
                icon={e.git ? 'git-branch-outline' : 'folder-outline'}
                label={e.name}
                value={e.git ? 'git' : undefined}
                disabled={busy}
                divided={i > 0 || !!listing.parent}
                onPress={() => onOpen(e.path)}
              />
            ))}
            {listing.entries.length === 0 ? (
              <Text style={{ color: colors.sub, padding: 16 }}>{t('No folders here.')}</Text>
            ) : null}
          </Group>
        ) : null}
        {error ? <ErrorText text={error} /> : null}
      </ScrollView>
      {listing ? (
        <PrimaryButton
          label={t('Use this folder')}
          busy={busy}
          disabled={busy}
          onPress={() => onUse(listing.path)}
        />
      ) : null}
    </View>
  );
}
