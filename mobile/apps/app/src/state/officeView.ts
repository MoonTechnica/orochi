import { useSyncExternalStore } from 'react';
import type { OfficeFilter } from '@orochi/core';

// The chosen filter lives as long as the app does, so leaving the Office and coming back keeps it.
let filter: OfficeFilter = 'all';
const subs = new Set<() => void>();

export const setOfficeFilter = (next: OfficeFilter) => {
  if (next === filter) return;
  filter = next;
  subs.forEach((f) => f());
};

export function useOfficeFilter(): [OfficeFilter, (f: OfficeFilter) => void] {
  const value = useSyncExternalStore(
    (fn) => {
      subs.add(fn);
      return () => subs.delete(fn);
    },
    () => filter,
    () => filter,
  );
  return [value, setOfficeFilter];
}
