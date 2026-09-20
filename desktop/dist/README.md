# Looking at the window without building it

`preview.html` renders the same `app.js` and `app.css` the window uses, against the recorded
view output in `../tests/*.json`, with `window.__TAURI__` stubbed in `preview-data.js`. It is
how the page's *appearance* is checked — the app itself gets these answers from the core, and
its behavior is checked by `../tests/ui.test.mjs` and the Rust tests beside it.

```sh
cd desktop/dist && python3 -m http.server 8731   # then open /preview.html
```
