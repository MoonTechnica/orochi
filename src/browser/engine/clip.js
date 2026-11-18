((ref) => {
  const el = window.__orochiRefs && window.__orochiRefs.get(ref);
  if (!el || !el.isConnected) return JSON.stringify({ error: 'that ref is not on the page any more; take a new snapshot' });
  el.scrollIntoView({ block: 'center', inline: 'center' });
  const r = el.getBoundingClientRect();
  return JSON.stringify({ x: r.left, y: r.top, width: r.width, height: r.height, tag: el.tagName });
})(__REF__)
