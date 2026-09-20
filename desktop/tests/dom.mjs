// Just enough DOM for `app.js`, so the window's rendering can be tested against recorded view
// output without a browser. It is deliberately small: anything the app needs that is missing
// here shows up as a failure rather than as a silently different screen.
class Node {
  constructor(tag) {
    this.tagName = String(tag).toUpperCase();
    this.children = [];
    this.parent = null;
    this.attributes = new Map();
    this.dataset = {};
    this.style = {};
    this.listeners = new Map();
    this._text = "";
    this.hidden = false;
    this.open = false;
  }
  set className(value) { this.attributes.set("class", value); }
  get className() { return this.attributes.get("class") || ""; }
  set textContent(value) { this._text = String(value); this.children = []; }
  get textContent() {
    return this.children.length ? this.children.map((c) => c.textContent).join("") : this._text;
  }
  get childElementCount() { return this.children.filter((c) => c instanceof Node).length; }
  append(...nodes) {
    for (const node of nodes) {
      const child = node instanceof Node || node instanceof TextNode ? node : new TextNode(String(node));
      child.parent = this;
      this.children.push(child);
    }
  }
  replaceChildren(...nodes) { this.children = []; this._text = ""; this.append(...nodes); }
  setAttribute(name, value) { this.attributes.set(name, String(value)); }
  getAttribute(name) { return this.attributes.get(name) ?? null; }
  addEventListener(name, handler) {
    if (!this.listeners.has(name)) this.listeners.set(name, []);
    this.listeners.get(name).push(handler);
  }
  dispatch(name, event = {}) {
    // Events reach the document, as they do in a browser. Without this a handler that opens
    // something and a document handler that closes what was not clicked look independent here
    // and cancel each other in the window. The app delegates only from the document, so
    // walking the ancestors in between would buy nothing.
    let stopped = false;
    const carried = {
      preventDefault() {},
      stopPropagation() { stopped = true; },
      target: this,
      ...event,
    };
    for (const handler of this.listeners.get(name) || []) handler(carried);
    if (stopped) return;
    for (const handler of document.listeners.get(name) || []) handler(carried);
  }
  // A node's own subtree, not every node ever made: a stale row from an earlier test would
  // otherwise answer for this one.
  querySelectorAll(selector) {
    const found = [];
    const walk = (node) => {
      for (const child of node.children) {
        if (!(child instanceof Node)) continue;
        if (matches(child, selector)) found.push(child);
        walk(child);
      }
    };
    walk(this);
    return found;
  }
  requestSubmit() { this.dispatch("submit"); }
  // Enough of <dialog> for the panels: opening sets `open`, closing clears it and fires the
  // event the page listens for.
  showModal() { this.open = true; this.hidden = false; }
  close() {
    if (!this.open) return;
    this.open = false;
    this.hidden = true;
    this.dispatch("close");
  }
  contains(node) {
    if (node === this) return true;
    return this.children.some((child) => child instanceof Node && child.contains(node));
  }
  // A flat, readable view of what was drawn, for assertions.
  render(depth = 0) {
    // What a person would see: a field shows its value, not its (empty) text.
    const own = this.children.length ? "" : this._text || this.value || "";
    const head = `${"  ".repeat(depth)}${this.tagName.toLowerCase()}${this.className ? "." + this.className : ""}${own ? " " + own : ""}`;
    return [head, ...this.children.map((c) => c.render(depth + 1))].filter(Boolean).join("\n");
  }
}
class TextNode {
  constructor(text) { this._text = text; }
  get textContent() { return this._text; }
  render(depth) { return this._text.trim() ? `${"  ".repeat(depth)}${this._text}` : ""; }
}
/// `tag`, `.class`, and the two together or stacked (`.said.system`), which is as much as the
/// page asks for. A selector the page uses and this does not understand would silently match
/// nothing, so anything else is refused rather than guessed at.
function matches(node, selector) {
  const parts = selector.trim().split(/(?=\.)/).filter(Boolean);
  return parts.every((part) => {
    if (part.startsWith(".")) {
      return node.className.split(/\s+/).includes(part.slice(1));
    }
    if (!/^[a-z][a-z0-9]*$/i.test(part)) {
      throw new Error(`the page asked for a selector this shim does not understand: ${selector}`);
    }
    return node.tagName.toLowerCase() === part.toLowerCase();
  });
}

export const document = {
  all: new Set(),
  byId: new Map(),
  createElement(tag) {
    const node = new Node(tag);
    document.all.add(node);
    return node;
  },
  createTextNode(value) { return new TextNode(String(value)); },
  listeners: new Map(),
  addEventListener(name, handler) {
    if (!document.listeners.has(name)) document.listeners.set(name, []);
    document.listeners.get(name).push(handler);
  },
  getElementById(id) {
    if (!document.byId.has(id)) {
      const node = document.createElement("div");
      node.id = id;
      document.byId.set(id, node);
    }
    return document.byId.get(id);
  },
  // The page's own tree: every element the test registered, and their descendants. Registered
  // elements nest, so the same node is reachable twice and is counted once.
  querySelectorAll(selector) {
    const roots = [...document.byId.values()];
    const found = new Set(roots.filter((n) => matches(n, selector)));
    for (const root of roots) for (const n of root.querySelectorAll(selector)) found.add(n);
    return [...found];
  },
};

/// Registers the ids `index.html` declares, so the app finds the same elements it would in a
/// window rather than inventing them one at a time. `hidden` is read from the markup, because
/// an element the page starts hidden behaves differently from one it does not.
export function page(ids, markup = "") {
  const startsHidden = new Set(
    [...markup.matchAll(/<[^>]*\bid="([^"]+)"[^>]*>/g)]
      .filter((m) => /\shidden(\s|>)/.test(m[0]))
      .map((m) => m[1]),
  );
  for (const id of ids) {
    const node = new Node(id === "message" ? "textarea" : id === "panel" ? "dialog" : "div");
    node.id = id;
    node.value = "";
    node.hidden = startsHidden.has(id);
    document.all.add(node);
    document.byId.set(id, node);
  }
  // The markup's own nesting, so an element really is inside the one that declares it: two of
  // the app's handlers close a menu when the click was outside the box it belongs to, and
  // without a tree `contains` answers for nothing. Anonymous buttons are built here too, or
  // the app would find no tabs and no screens to switch between.
  const empty = new Set(["meta", "link", "br", "img", "input", "hr"]);
  const stack = [];
  for (const [full, closing, tag, attributes] of markup.matchAll(/<(\/?)([a-z0-9]+)([^>]*)>/g)) {
    if (closing) {
      const at = stack.findLastIndex((frame) => frame.tag === tag);
      if (at >= 0) stack.length = at;
      continue;
    }
    const id = attributes.match(/\bid="([^"]+)"/)?.[1];
    let node = id ? document.byId.get(id) ?? null : null;
    if (!id && tag === "button") {
      node = new Node("button");
      const className = attributes.match(/\bclass="([^"]+)"/)?.[1];
      if (className) node.className = className;
      for (const [, key, value] of attributes.matchAll(/\bdata-([a-z-]+)="([^"]+)"/g)) {
        node.dataset[key.replace(/-(\w)/g, (_, c) => c.toUpperCase())] = value;
      }
      for (const [, key, value] of attributes.matchAll(/\b(aria-[a-z]+)="([^"]+)"/g)) {
        node.setAttribute(key, value);
      }
      document.all.add(node);
    }
    if (node) {
      const parent = stack.map((frame) => frame.node).filter(Boolean).pop();
      if (parent && parent !== node) parent.append(node);
    }
    if (!empty.has(tag) && !full.endsWith("/>")) stack.push({ tag, node });
  }
}

export { Node };
