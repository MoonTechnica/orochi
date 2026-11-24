import React, { useEffect, useMemo, useRef, useState } from 'react';
import { Platform, Pressable, Text, TextInput, View } from 'react-native';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import { applyMention, mentionQuery, type Candidate } from '@orochi/core';
import { getDraft, setDraft } from '../state/drafts';
import { usePrefs } from '../state/prefs';
import { usePeople } from './people/People';
import { WhoFace } from './people/WhoFace';
import { Icon } from './Icon';

export function Composer({
  placeholder,
  safeArea = true,
  working,
  candidates,
  onSend,
  onInterrupt,
  also,
  focus,
  initial,
  draftKey,
}: {
  placeholder: string;
  safeArea?: boolean;
  working?: boolean;
  candidates: (query: string) => Candidate[];
  onSend: (text: string) => Promise<void> | void;
  onInterrupt?: () => void;
  also?: { label: string; value: boolean; onChange: (v: boolean) => void };
  /** Focus the input once it is mounted (a reply to a mention) and start it with `initial`. */
  focus?: boolean;
  initial?: string;
  /** Keeps the unsent text under this key while the app runs (one composer per thread). */
  draftKey?: string;
}) {
  const { colors, t } = usePrefs();
  const people = usePeople();
  const insets = useSafeAreaInsets();
  const [value, setValue] = useState(() => getDraft(draftKey));
  const [caret, setCaret] = useState(() => getDraft(draftKey).length);
  useEffect(() => setDraft(draftKey, value), [draftKey, value]);
  const [busy, setBusy] = useState(false);
  const input = useRef<React.ElementRef<typeof TextInput>>(null);
  useEffect(() => {
    if (!focus) return;
    if (initial) {
      setValue(initial);
      setCaret(initial.length);
    }
    // After the screen's own transition: focusing earlier is lost on iOS.
    const timer = setTimeout(() => input.current?.focus(), 350);
    return () => clearTimeout(timer);
  }, [focus, initial]);
  const q = useMemo(
    () => mentionQuery(value, Platform.OS === 'web' || caret === 0 ? value.length : caret),
    [value, caret],
  );
  const hits = useMemo(() => (q ? candidates(q.query).slice(0, 8) : []), [q, candidates]);

  const send = async () => {
    const text = value.trim();
    if (!text || busy) return;
    setBusy(true);
    try {
      await onSend(text);
      setValue('');
      setCaret(0);
    } finally {
      setBusy(false);
    }
  };

  return (
    <View
      style={{
        borderTopWidth: 1,
        borderTopColor: colors.line,
        backgroundColor: colors.card,
        paddingBottom: safeArea ? Math.max(insets.bottom, 8) : 8,
      }}
    >
      {hits.length > 0 ? (
        <View accessibilityLabel={t('Mentions')}>
          {hits.map((c) => (
            <Pressable
              key={c.name}
              onPress={() => {
                if (!q) return;
                const next = applyMention(value, q, c.name);
                setValue(next.value);
                setCaret(next.caret);
                input.current?.focus();
              }}
              style={{
                minHeight: 48,
                flexDirection: 'row',
                alignItems: 'center',
                gap: 10,
                paddingHorizontal: 14,
              }}
            >
              {c.channel ? (
                <Icon name="people" size={26} color={colors.sub} />
              ) : (
                <WhoFace
                  who={people.who(c.name)}
                  size={30}
                  state={c.live ? 'working' : undefined}
                />
              )}
              <Text style={{ color: colors.text, fontWeight: '700' }}>@{c.name}</Text>
              <Text style={{ color: colors.sub, flex: 1 }} numberOfLines={1}>
                {c.channel ? t('everyone') : [c.agent, c.model].filter(Boolean).join(' · ')}
              </Text>
              <Text style={{ color: c.live ? colors.text : colors.sub, fontSize: 12 }}>
                {c.state}
              </Text>
            </Pressable>
          ))}
        </View>
      ) : null}
      <View
        style={{
          flexDirection: 'row',
          alignItems: 'flex-end',
          gap: 8,
          padding: 8,
          paddingBottom: also ? 0 : 8,
        }}
      >
        <TextInput
          ref={input}
          value={value}
          onChangeText={(text) => {
            setValue(text);
            setCaret(text.length);
          }}
          onSelectionChange={(e) => setCaret(e.nativeEvent.selection.start)}
          placeholder={placeholder}
          placeholderTextColor={colors.sub}
          testID="composer-input"
          multiline
          style={{
            flex: 1,
            minHeight: 44,
            maxHeight: 140,
            borderRadius: 12,
            borderWidth: 1,
            borderColor: colors.line,
            paddingHorizontal: 12,
            paddingTop: 10,
            paddingBottom: 10,
            color: colors.text,
            fontSize: 16,
            backgroundColor: colors.bg,
          }}
        />
        {working && onInterrupt ? (
          <Pressable
            onPress={onInterrupt}
            accessibilityRole="button"
            accessibilityLabel={t('Stop')}
            style={{
              width: 44,
              height: 44,
              borderRadius: 12,
              alignItems: 'center',
              justifyContent: 'center',
              backgroundColor: colors.danger,
            }}
          >
            <Icon name="stop" size={20} color={colors.onDanger} />
          </Pressable>
        ) : null}
        <Pressable
          onPress={send}
          disabled={!value.trim() || busy}
          accessibilityRole="button"
          accessibilityLabel={working ? t('Queue') : t('Send')}
          style={{
            width: 44,
            height: 44,
            borderRadius: 12,
            alignItems: 'center',
            justifyContent: 'center',
            backgroundColor: value.trim() ? colors.primary : colors.raised,
          }}
        >
          <Icon
            name={working ? 'time' : 'arrow-up'}
            size={22}
            color={value.trim() ? colors.onPrimary : colors.sub}
          />
        </Pressable>
      </View>
      {also ? (
        <Pressable
          onPress={() => also.onChange(!also.value)}
          accessibilityRole="checkbox"
          accessibilityState={{ checked: also.value }}
          accessibilityLabel={also.label}
          testID="also-channel"
          style={{
            minHeight: 40,
            flexDirection: 'row',
            alignItems: 'center',
            paddingHorizontal: 12,
            gap: 8,
          }}
        >
          <Icon
            name={also.value ? 'checkbox' : 'square-outline'}
            size={22}
            color={also.value ? colors.text : colors.sub}
          />
          <Text style={{ color: colors.text, fontSize: 14 }}>{also.label}</Text>
        </Pressable>
      ) : null}
    </View>
  );
}
