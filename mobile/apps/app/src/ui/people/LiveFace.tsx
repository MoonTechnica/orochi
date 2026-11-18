import React from 'react';
import type { PersonState, Who } from '@orochi/core';
import { Avatar } from '../office/Avatar';
import { WhoFace } from './WhoFace';

/**
 * A member with a live state: the Office's own avatar (ring, bob, blink) while it works or asks, which
 * pauses under reduced motion; the static face with the state's expression otherwise. Used only for
 * the few faces that say what someone is doing now (a thread bar, the thread panel), never in the
 * message list.
 */
export function LiveFace({
  who,
  state,
  size,
  moving = true,
}: {
  who: Who;
  state: PersonState;
  size: number;
  /** False keeps the Office avatar's ring without its motion. */
  moving?: boolean;
}) {
  if (who.you || (!who.role && !who.agent) || (state !== 'working' && state !== 'asking'))
    return <WhoFace who={who} size={size} state={state} />;
  return (
    <Avatar
      person={{
        role: who.role ?? '',
        lead: who.lead,
        provider: who.provider ?? '',
        agent: who.agent ?? '',
        state,
        speaking: false,
      }}
      size={size}
      still={!moving}
    />
  );
}
