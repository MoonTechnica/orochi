import { useSyncExternalStore } from 'react';
import type { OfficeFilter } from '@orochi/core';

// The chosen filter lives as long as the app does, so leaving the Office and coming back keeps it.
const filters = new Map<string | null, OfficeFilter>();
const subs = new Set<() => void>();

export function useOfficeFilter(key: string | null): [OfficeFilter, (f: OfficeFilter) => void] {
  const value = useSyncExternalStore(
    (fn) => {
      subs.add(fn);
      return () => subs.delete(fn);
    },
    () => filters.get(key) ?? 'all',
    () => filters.get(key) ?? 'all',
  );
  return [
    value,
    (next) => {
      if (next === (filters.get(key) ?? 'all')) return;
      filters.set(key, next);
      subs.forEach((f) => f());
    },
  ];
}
