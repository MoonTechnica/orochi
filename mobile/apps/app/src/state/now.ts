import { useEffect, useState } from 'react';

/** The current time, refreshed every `every` ms, so "4m ago" does not go stale on a screen left open. */
export function useNow(every = 30_000): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), every);
    return () => clearInterval(timer);
  }, [every]);
  return now;
}
