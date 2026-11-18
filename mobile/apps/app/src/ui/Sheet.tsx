import React, { useEffect, useState } from 'react';
import {
  KeyboardAvoidingView,
  Modal,
  Platform,
  Pressable,
  Text,
  TextInput,
  View,
} from 'react-native';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import { usePrefs } from '../state/prefs';

export interface SheetAction {
  label: string;
  onPress: () => void;
  danger?: boolean;
  selected?: boolean;
  id?: string;
}

/** A bottom sheet of actions; tapping the backdrop closes it. */
export function Sheet({
  visible,
  title,
  actions,
  onClose,
}: {
  visible: boolean;
  title?: string;
  actions: SheetAction[];
  onClose: () => void;
}) {
  const { colors, t } = usePrefs();
  const insets = useSafeAreaInsets();
  return (
    <Modal visible={visible} transparent animationType="fade" onRequestClose={onClose}>
      <View
        accessibilityViewIsModal
        style={{ flex: 1, backgroundColor: '#0006', justifyContent: 'flex-end' }}
      >
        <Pressable
          style={{ position: 'absolute', top: 0, bottom: 0, left: 0, right: 0 }}
          onPress={onClose}
          accessibilityRole="button"
          accessibilityLabel={t('Close menu')}
        />
        <View
          style={{
            backgroundColor: colors.card,
            borderTopLeftRadius: 12,
            borderTopRightRadius: 12,
            paddingBottom: insets.bottom + 8,
            maxHeight: '80%',
          }}
        >
          {title ? (
            <Text
              numberOfLines={1}
              style={{ color: colors.sub, fontSize: 13, padding: 16, paddingBottom: 6 }}
            >
              {title}
            </Text>
          ) : null}
          {actions.map((a, i) => (
            <Pressable
              key={a.id ?? `${i}-${a.label}`}
              accessibilityRole="button"
              accessibilityLabel={a.label}
              onPress={() => {
                onClose();
                a.onPress();
              }}
              style={{
                minHeight: 52,
                paddingHorizontal: 20,
                justifyContent: 'center',
                borderTopWidth: 1,
                borderTopColor: colors.line,
              }}
            >
              <Text
                style={{
                  color: a.danger ? colors.dangerInk : colors.text,
                  fontSize: 17,
                  fontWeight: a.selected ? '700' : '400',
                }}
              >
                {a.selected ? '✓ ' : ''}
                {a.label}
              </Text>
            </Pressable>
          ))}
          <Pressable
            onPress={onClose}
            accessibilityRole="button"
            style={{
              minHeight: 52,
              justifyContent: 'center',
              paddingHorizontal: 20,
              borderTopWidth: 1,
              borderTopColor: colors.line,
            }}
          >
            <Text style={{ color: colors.sub, fontSize: 17 }}>{t('Cancel')}</Text>
          </Pressable>
        </View>
      </View>
    </Modal>
  );
}

/** A one-line text prompt (Alert.prompt exists on iOS only). */
export function TextPrompt({
  visible,
  title,
  initial,
  placeholder,
  confirm,
  inline,
  onSubmit,
  onClose,
}: {
  visible: boolean;
  title: string;
  initial?: string;
  placeholder?: string;
  confirm: string;
  /** Drawn in place, not in a modal: iOS does not deliver touches to a modal opened over another. */
  inline?: boolean;
  onSubmit: (text: string) => void;
  onClose: () => void;
}) {
  const { colors, t } = usePrefs();
  const [value, setValue] = useState(initial ?? '');
  useEffect(() => {
    if (visible) setValue(initial ?? '');
  }, [visible, initial]);
  const card = (
    <View style={{ backgroundColor: colors.card, borderRadius: 12, padding: 16, gap: 12 }}>
      <Text style={{ color: colors.text, fontSize: 17, fontWeight: '700' }}>{title}</Text>
      <TextInput
        value={value}
        onChangeText={setValue}
        placeholder={placeholder}
        placeholderTextColor={colors.sub}
        autoFocus
        accessibilityLabel={title}
        onSubmitEditing={() => {
          onClose();
          onSubmit(value.trim());
        }}
        style={{
          minHeight: 46,
          borderWidth: 1,
          borderColor: colors.line,
          borderRadius: 10,
          paddingHorizontal: 12,
          color: colors.text,
          fontSize: 16,
        }}
      />
      <View style={{ flexDirection: 'row', justifyContent: 'flex-end', gap: 8 }}>
        <Pressable
          onPress={onClose}
          style={{ minHeight: 44, justifyContent: 'center', padding: 10 }}
        >
          <Text style={{ color: colors.sub, fontSize: 16 }}>{t('Cancel')}</Text>
        </Pressable>
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={confirm}
          onPress={() => {
            onClose();
            onSubmit(value.trim());
          }}
          style={{ minHeight: 44, justifyContent: 'center', padding: 10 }}
        >
          <Text style={{ color: colors.text, fontSize: 16, fontWeight: '700' }}>{confirm}</Text>
        </Pressable>
      </View>
    </View>
  );
  if (inline) return visible ? card : null;
  return (
    <Modal visible={visible} transparent animationType="fade" onRequestClose={onClose}>
      <KeyboardAvoidingView
        behavior={Platform.OS === 'ios' ? 'padding' : undefined}
        style={{ flex: 1, backgroundColor: '#0006', justifyContent: 'center', padding: 24 }}
      >
        {card}
      </KeyboardAvoidingView>
    </Modal>
  );
}
