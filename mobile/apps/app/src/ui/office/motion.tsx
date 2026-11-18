import React, { createContext, useContext, useEffect, useMemo, useSyncExternalStore } from 'react';
import { AppState } from 'react-native';
import { useReducedMotion } from 'react-native-reanimated';
import { motionAllowed, type MotionState } from '@orochi/core';

/**
 * Who may move. The Office screen owns one store (which rooms the list shows, whether the screen is
 * crowded, whether the app is in front); a person subscribes to one boolean, so a scroll re-renders
 * only the rooms whose answer changed. Outside the Office (Home's card, the sheets) there is no store
 * and the answer is yes.
 */
export interface MotionStore {
  get: () => MotionState;
  set: (patch: Partial<MotionState>) => void;
  subscribe: (fn: () => void) => () => void;
}

export function createMotionStore(): MotionStore {
  let state: MotionState = {
    foreground: AppState.currentState !== 'background' && AppState.currentState !== 'inactive',
    crowded: false,
    visible: new Set(),
  };
  const subs = new Set<() => void>();
  return {
    get: () => state,
    set: (patch) => {
      const same =
        (patch.foreground === undefined || patch.foreground === state.foreground) &&
        (patch.crowded === undefined || patch.crowded === state.crowded) &&
        (patch.visible === undefined ||
          (patch.visible.size === state.visible.size &&
            [...patch.visible].every((id) => state.visible.has(id))));
      if (same) return;
      state = { ...state, ...patch };
      subs.forEach((f) => f());
    },
    subscribe: (fn) => {
      subs.add(fn);
      return () => subs.delete(fn);
    },
  };
}

/** A store for one screen, kept in step with the app's foreground state. */
export function useMotionStore(): MotionStore {
  const store = useMemo(createMotionStore, []);
  useEffect(() => {
    const sub = AppState.addEventListener('change', (s) =>
      store.set({ foreground: s !== 'background' && s !== 'inactive' }),
    );
    return () => sub.remove();
  }, [store]);
  return store;
}

const StoreContext = createContext<MotionStore | null>(null);
const RoomContext = createContext<string | null>(null);

export const MotionProvider = StoreContext.Provider;

/** Everyone inside is in this room. */
export function MotionRoom({ id, children }: { id: string; children: React.ReactNode }) {
  return <RoomContext.Provider value={id}>{children}</RoomContext.Provider>;
}

const never = () => () => {};

/** True when this person may pulse, bob, blink and talk: motion is on, in front, not crowded, in view. */
export function useOfficeMotion(): boolean {
  const reduced = useReducedMotion();
  const store = useContext(StoreContext);
  const room = useContext(RoomContext);
  const allowed = useSyncExternalStore(
    store ? store.subscribe : never,
    () => (store ? motionAllowed(store.get(), room) : true),
    () => true,
  );
  return allowed && !reduced;
}
