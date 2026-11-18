((ref, force) => {
  const el = window.__orochiRefs && window.__orochiRefs.get(ref);
  if (!el || !el.isConnected) return JSON.stringify({ error: 'that ref is not on the page any more; take a new snapshot' });
  const has = () => document.activeElement === el || el.contains(document.activeElement);
  if (force && !has()) el.focus();
  return JSON.stringify({ ok: has() });
})(__REF__, __FORCE__)
