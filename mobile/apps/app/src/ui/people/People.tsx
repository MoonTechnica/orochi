import React, { createContext, useContext, useMemo } from 'react';
import type { Member, ThreadRow } from '@orochi/protocol';
import {
  isAnonymous,
  leadRoleOf,
  personStateOf,
  whoNamed,
  whoOf,
  type PersonState,
  type Voice,
  type Who,
} from '@orochi/core';

interface Roster {
  who: (name: string) => Who;
  member: (name: string) => Member | undefined;
  voice: (v: Voice) => Who;
  state: (name: string) => PersonState;
}

const fallback: Roster = {
  who: (name) => whoOf({ name }, null),
  member: () => undefined,
  voice: (v) =>
    whoOf({ name: v.who, role: v.role, agent: v.agent, provider: v.provider, mine: v.mine }, null),
  state: () => 'reported',
};

const NONE: Member[] = [];
const Ctx = createContext<Roster>(fallback);

/** Who the names in one conversation are (its members and its lead), for every face drawn in it. */
export function PeopleProvider({
  thread: row,
  members = NONE,
  children,
}: {
  thread: ThreadRow | null | undefined;
  members?: Member[];
  children: React.ReactNode;
}) {
  const value = useMemo<Roster>(() => {
    const leadRole = leadRoleOf(row);
    const find = (name: string) => members.find((m) => m.name.toLowerCase() === name.toLowerCase());
    return {
      who: (name) => whoNamed(name, members, row),
      member: find,
      voice: (v) => {
        const w = whoOf(
          { name: v.who, role: v.role, agent: v.agent, provider: v.provider, mine: v.mine },
          leadRole,
        );
        // A post that names no role or agent (older rows) is still the member or lead it is signed as.
        if (!isAnonymous(w) || !v.who) return w;
        const named = whoNamed(v.who, members, row);
        return named.agent ? named : w;
      },
      state: (name) => personStateOf(find(name)),
    };
  }, [row, members]);
  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}

export const usePeople = () => useContext(Ctx);
