import React from 'react';
import { useLocalSearchParams } from 'expo-router';
import { ConversationView } from '../../../../src/ui/Conversation';
import { HEADER } from '../../../../src/ui/Screen';

export default function Conversation() {
  const { id, reply } = useLocalSearchParams<{ id: string; reply?: string }>();
  return <ConversationView id={id} reply={reply} />;
}
export { HEADER };
