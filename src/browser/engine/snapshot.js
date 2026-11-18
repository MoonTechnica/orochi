(() => {
  const LIMIT = __LIMIT__;
  let store = window.__orochiRefs;
  if (!store) {
    store = new Map();
    Object.defineProperty(window, '__orochiRefs', { value: store, enumerable: false });
  }
  store.clear();
  const SKIP = new Set(['SCRIPT', 'STYLE', 'NOSCRIPT', 'TEMPLATE', 'HEAD', 'META', 'LINK', 'SVG', 'CANVAS']);
  const WHOLE = new Set(['link', 'button', 'heading', 'option', 'tab', 'menuitem']);
  const clean = (t, n = 160) => (t || '').replace(/\s+/g, ' ').trim().slice(0, n);
  const roleOf = (el) => {
    const tag = el.tagName;
    const explicit = el.getAttribute('role');
    if (explicit && /^(button|link|tab|menuitem|switch|checkbox|radio|textbox|searchbox|combobox|option)$/.test(explicit)) return explicit;
    if (tag === 'A') return el.hasAttribute('href') ? 'link' : null;
    if (tag === 'BUTTON') return 'button';
    if (tag === 'SELECT') return 'combobox';
    if (tag === 'TEXTAREA') return 'textbox';
    if (tag === 'INPUT') {
      const t = (el.getAttribute('type') || 'text').toLowerCase();
      if (t === 'hidden') return null;
      if (['button', 'submit', 'reset', 'image'].includes(t)) return 'button';
      if (t === 'checkbox') return 'checkbox';
      if (t === 'radio') return 'radio';
      if (t === 'search') return 'searchbox';
      if (t === 'file') return 'button';
      return 'textbox';
    }
    if (el.isContentEditable && el.parentElement && !el.parentElement.isContentEditable) return 'textbox';
    if (/^H[1-6]$/.test(tag)) return 'heading';
    return null;
  };
  const nameOf = (el, role) => {
    const aria = el.getAttribute('aria-label');
    if (aria) return clean(aria);
    const by = el.getAttribute('aria-labelledby');
    if (by) {
      const t = by.split(/\s+/).map((id) => document.getElementById(id)).filter(Boolean).map((n) => n.textContent).join(' ');
      if (clean(t)) return clean(t);
    }
    if (el.labels && el.labels.length) return clean(Array.from(el.labels).map((l) => l.textContent).join(' '));
    const tag = el.tagName;
    if (tag === 'INPUT' && ['button', 'submit', 'reset'].includes((el.type || '').toLowerCase())) return clean(el.value);
    if (tag === 'INPUT' || tag === 'TEXTAREA') return clean(el.placeholder || el.title || el.name);
    if (tag === 'IMG') return clean(el.alt);
    return clean(el.innerText || el.textContent) || clean(el.title);
  };
  const shown = (el) => {
    if (el.hidden || el.getAttribute('aria-hidden') === 'true') return false;
    const s = getComputedStyle(el);
    return s.display !== 'none' && s.visibility !== 'hidden';
  };
  let out = '';
  let n = 0;
  const walk = (el, depth) => {
    if (out.length > LIMIT) return;
    if (SKIP.has(el.tagName.toUpperCase()) || !shown(el)) return;
    const role = roleOf(el);
    const pad = '  '.repeat(depth);
    if (role) {
      let line = `${pad}- ${role}`;
      const name = nameOf(el, role);
      if (name) line += ` ${JSON.stringify(name)}`;
      if (role === 'heading') line += ` [level=${el.tagName[1]}]`;
      if (role === 'textbox' || role === 'searchbox' || role === 'combobox') {
        const v = el.value !== undefined ? el.value : el.textContent;
        if (v) line += ` value=${JSON.stringify(clean(v, 80))}`;
      }
      if (role === 'checkbox' || role === 'radio' || role === 'switch') line += el.checked ? ' [checked]' : '';
      if (el.disabled) line += ' [disabled]';
      if (role !== 'heading') {
        n += 1;
        const ref = `e${n}`;
        store.set(ref, el);
        line += ` [ref=${ref}]`;
      }
      out += line + '\n';
      if (WHOLE.has(role) || role === 'textbox' || role === 'searchbox') return;
      for (const child of el.children) walk(child, depth + 1);
      return;
    }
    if (el.tagName === 'IMG') {
      const alt = clean(el.alt);
      if (alt) out += `${pad}- img ${JSON.stringify(alt)}\n`;
      return;
    }
    let next = depth;
    if (['UL', 'OL', 'NAV', 'MAIN', 'FORM', 'TABLE', 'LI', 'SECTION', 'ARTICLE'].includes(el.tagName)) {
      out += `${pad}- ${el.tagName.toLowerCase()}\n`;
      next += 1;
    }
    for (const node of el.childNodes) {
      if (node.nodeType === 3) {
        const t = clean(node.textContent, 200);
        if (t) out += `${'  '.repeat(next)}- text ${JSON.stringify(t)}\n`;
      } else if (node.nodeType === 1) {
        walk(node, next);
      }
    }
  };
  if (document.body) walk(document.body, 0);
  if (out.length > LIMIT) out = out.slice(0, LIMIT) + '\n… (the page is longer than this)\n';
  return JSON.stringify({ text: out, refs: n });
})()
