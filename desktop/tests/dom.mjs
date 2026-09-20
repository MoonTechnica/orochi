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
    for (const handler of this.listeners.get(name) || []) handler({ preventDefault() {}, target: this, ...event });
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
  contains(node) {
    if (node === this) return true;
    return this.children.some((child) => child instanceof Node && child.contains(node));
  }
  // A flat, readable view of what was drawn, for assertions.
  render(depth = 0) {
    const own = this.children.length ? "" : this._text;
    const head = `${"  ".repeat(depth)}${this.tagName.toLowerCase()}${this.className ? "." + this.className : ""}${own ? " " + own : ""}`;
    return [head, ...this.children.map((c) => c.render(depth + 1))].filter(Boolean).join("\n");
  }
}
class TextNode {
  constructor(text) { this._text = text; }
  get textContent() { return this._text; }
  render(depth) { return this._text.trim() ? `${"  ".repeat(depth)}${this._text}` : ""; }
}
function matches(node, selector) {
  if (selector.startsWith(".")) return node.className.split(/\s+/).includes(selector.slice(1));
  return node.tagName.toLowerCase() === selector.toLowerCase();
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
  // The page's own tree: every element the test registered, and their descendants.
  querySelectorAll(selector) {
    const roots = [...document.byId.values()];
    const found = roots.filter((n) => matches(n, selector));
    for (const root of roots) found.push(...root.querySelectorAll(selector));
    return found;
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
    const node = new Node(id === "message" ? "textarea" : "div");
    node.id = id;
    node.value = "";
    node.hidden = startsHidden.has(id);
    document.all.add(node);
    document.byId.set(id, node);
  }
  // The buttons the markup declares, under the container they are declared in. Without them
  // the app would find no tabs and no screens to switch between — and the test would be
  // passing over a page that does not exist.
  let container = null;
  for (const tag of markup.match(/<(div|button|section|aside|main|header)\b[^>]*>/g) || []) {
    const id = tag.match(/\bid="([^"]+)"/)?.[1];
    if (!tag.startsWith("<button")) {
      if (id && document.byId.has(id)) container = document.byId.get(id);
      continue;
    }
    // A button with its own id was registered above; only the anonymous ones belong here.
    if (id) continue;
    const node = new Node("button");
    const className = tag.match(/\bclass="([^"]+)"/)?.[1];
    if (className) node.className = className;
    for (const [, key, value] of tag.matchAll(/\bdata-([a-z-]+)="([^"]+)"/g)) {
      node.dataset[key.replace(/-(\w)/g, (_, c) => c.toUpperCase())] = value;
    }
    for (const [, key, value] of tag.matchAll(/\b(aria-[a-z]+)="([^"]+)"/g)) {
      node.setAttribute(key, value);
    }
    document.all.add(node);
    if (container) container.append(node);
  }
}

export { Node };
