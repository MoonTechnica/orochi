# Looking at the window without building it

`preview.html` is generated from `index.html` by `../tests/make-preview.mjs`, so the page that
is looked at is the page that ships — a hand-copied one drifts, and a missing element takes the
whole script down with it. It renders the real `app.js` and `app.css` against the recorded view
output in `../tests/*.json`, with `window.__TAURI__` stubbed in `preview-data.js`.

```sh
node desktop/tests/make-preview.mjs
cd desktop/dist && python3 -m http.server 8731   # then open /preview.html
```

The window's *behavior* is checked by `../tests/ui.test.mjs` and the Rust tests beside it; this
is how its *appearance* is checked.
