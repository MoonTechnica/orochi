import React from 'react';
import { ScrollView, Text, View } from 'react-native';
import {
  ACCESSORIES,
  STATES,
  faceOf,
  lookFor,
  seedOf,
  type Kind,
  type Person,
  whoOf,
  type PersonState,
} from '@orochi/core';
import { usePrefs } from '../src/state/prefs';
import { Header } from '../src/ui/Screen';
import { Avatar, MiniAvatar } from '../src/ui/office/Avatar';
import { WhoFace } from '../src/ui/people/WhoFace';

const person = (role: string, agent: string, state: PersonState, lead = false): Person =>
  ({
    role,
    agent,
    provider: 'anthropic',
    lead,
    state,
    speaking: false,
    tasks: [],
  }) as unknown as Person;

const ROLES: { kind: Kind; role: string; lead: boolean }[] = [
  { kind: 'lead', role: 'lead', lead: true },
  { kind: 'implementer', role: 'implementer', lead: false },
  { kind: 'reviewer', role: 'reviewer', lead: false },
  { kind: 'other', role: 'designer', lead: false },
];

/**
 * Every part of the characters at once, for looking at them: not linked from anywhere, reached
 * by the address /avatars. A grid per role, each state, the sizes, the sub-agent, the accessories.
 */
export default function Avatars() {
  const { colors } = usePrefs();
  const label = (text: string) => (
    <Text style={{ color: colors.sub, fontSize: 11, fontWeight: '700', marginTop: 14 }}>
      {text.toUpperCase()}
    </Text>
  );
  const row = { flexDirection: 'row', flexWrap: 'wrap', gap: 10, alignItems: 'flex-end' } as const;
  return (
    <View style={{ flex: 1, backgroundColor: colors.bg }}>
      <Header title="Characters" />
      <ScrollView contentContainerStyle={{ padding: 16, paddingBottom: 60 }}>
        {label('states')}
        <View style={row}>
          {STATES.map((s) => (
            <View key={s} style={{ alignItems: 'center', gap: 4 }}>
              <Avatar person={person('implementer', 'claude', s)} size={64} />
              <Text style={{ color: colors.sub, fontSize: 10 }}>
                {s} · {faceOf(s, false).expression}
              </Text>
            </View>
          ))}
          <View style={{ alignItems: 'center', gap: 4 }}>
            <Avatar
              person={{ ...person('implementer', 'claude', 'working'), speaking: true }}
              size={64}
            />
            <Text style={{ color: colors.sub, fontSize: 10 }}>talking</Text>
          </View>
        </View>
        {label('sizes (40 is the Home card)')}
        <View style={row}>
          {[28, 40, 56, 72, 96].map((n) => (
            <Avatar key={n} person={person('reviewer', 'gemini', 'reported')} size={n} still />
          ))}
          <MiniAvatar title="Read the spec" agent="claude" size={16} />
          <MiniAvatar title="Run the tests" agent="claude" size={22} />
          <MiniAvatar title="Search" agent="codex" size={30} />
        </View>
        {ROLES.map(({ kind, role, lead }) => (
          <View key={kind}>
            {label(`${kind} · ${ACCESSORIES[kind].join(', ')}`)}
            <View style={row}>
              {Array.from({ length: 24 }, (_, i) => {
                const agent = `agent-${i}`;
                const state = STATES[i % STATES.length]!;
                void lookFor(seedOf({ role, agent }), kind);
                return <Avatar key={i} person={person(role, agent, state, lead)} size={52} still />;
              })}
            </View>
          </View>
        ))}
        {label(
          'in the conversation: you, the lead, a member, a reviewer, an unplaced speaker (34 and 22 px)',
        )}
        <View style={row}>
          {[34, 22].map((n) => (
            <React.Fragment key={n}>
              <WhoFace who={whoOf({ name: 'you', mine: true }, 'lead')} size={n} />
              <WhoFace
                who={whoOf(
                  { name: 'lead', role: 'lead', agent: 'claude', provider: 'anthropic' },
                  'lead',
                )}
                size={n}
              />
              <WhoFace
                who={whoOf(
                  {
                    name: 'implementer',
                    role: 'implementer',
                    agent: 'opencode',
                    provider: 'opencode',
                  },
                  'lead',
                )}
                size={n}
              />
              <WhoFace
                who={whoOf(
                  { name: 'reviewer', role: 'reviewer', agent: 'codex', provider: 'openai' },
                  'lead',
                )}
                size={n}
                state="asking"
              />
              <WhoFace who={whoOf({ name: 'mailbox' }, 'lead')} size={n} />
            </React.Fragment>
          ))}
        </View>
        {label('with the provider badge and an image mark')}
        <View style={row}>
          {['anthropic', 'openai', 'google', 'opencode', 'other'].map((p) => (
            <Avatar
              key={p}
              person={{ ...person('implementer', p, 'working'), provider: p }}
              size={64}
              still
            />
          ))}
        </View>
      </ScrollView>
    </View>
  );
}
