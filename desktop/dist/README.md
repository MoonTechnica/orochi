# Looking at the window without building it

`preview.html` is generated from `index.html` by `../tests/make-preview.mjs`, so the page that
is looked at is the page that ships — a hand-copied one drifts, and a missing element takes the
whole script down with it. It renders the real `app.js` and `app.css` against the recorded view
output in `../tests/*.json`, with `window.__TAURI__` stubbed in `preview-data.js`, and the Files pane's per-folder answers in
`preview-files.js`, which the same script generates from `../tests/files.json`.

```sh
node desktop/tests/make-preview.mjs
cd desktop/dist && python3 -m http.server 8731   # then open /preview.html
```

The window's *behavior* is checked by `../tests/ui.test.mjs` and the Rust tests beside it; this
is how its *appearance* is checked.

## Files in conversation messages

Screenshots can be returned as `![Screenshot](artifacts/screenshot.png)` or
`[Screenshot](artifacts/screenshot.png)`. Both show an inline image; clicking the image
opens a larger preview (Close or Escape dismisses it). Relative paths resolve from the
thread's working folder. Absolute paths and `file:///` URLs inside that folder also work;
use `<...>` around paths with spaces, or URL-encode them.

Video and audio links show native playback controls without autoplay. Other file links
open the Files pane, including optional `:line` or `#Lline` positions. Local previews use
a bounded Rust read (16 MiB per file) and reject traversal, `.git`, and symlinks outside
the working folder. Missing, oversized, or unsupported files keep their file action.

HTTPS media and HTTP media on `localhost`, `127.0.0.1`, or `[::1]` can also display.
Use image Markdown for URLs without an image extension. Playback formats depend on the
system webview. File contents are read from disk when shown and are not added to the store.
