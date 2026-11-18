# Servo spike, 2026-11-17

What was run: `browser-servo/` (`orochi-servo <url> <out.png>`), Servo 0.8.0 pinned with
`default-features = false, features = ["bundled", "js_jit"]`, a `SoftwareRenderingContext` of
1024×768, on this Mac (Rust 1.96.0, no `cmake`). It loads a URL, waits for `LoadStatus::Complete`
and calls `WebView::take_screenshot`. Part of `servo-browser-design.md` §9; nothing else in
that document has been run.

| Item (§9) | Result |
|---|---|
| 1. Builds | **Yes, first try.** `cargo build` 1 m 26 s, `cargo build --release` 1 m 47 s on a warm registry. Debug binary 315 MB, release binary 121.6 MB (unstripped) |
| 2. Renders | `example.com`: correct, including Chinese, Arabic and Cyrillic text. Wikipedia (*Kitesurfing*): layout, fonts, links and the photograph correct; **several SVG icons drawn as black squares** and the log says `unit 1 GLD_TEXTURE_INDEX_2D is unloadable … using zero texture`. GitHub (`servo/servo`): layout, fonts and icons correct, file-list rows still skeleton placeholders at the time of the shot. Hacker News: file written, **not looked at** |
| 3. Speed | Release: load complete in 0.5–1.1 s, screenshot written 1.1–1.8 s after start, process total 1.1–3.1 s for Wikipedia, Hacker News and GitHub, **one frame each, cold**. This is not a frame rate; read-back and encode of a stream were not measured. **Debug is unusable**: Wikipedia did not produce a screenshot in 90 s |
| 4. The user's login site | **Not run** (needs the site and an account; the owner's decision) |
| 5. IME | Not run |
| 6. Cookie across processes | Not run |
| 7. Gestures, `<select>`, dialogs, popups, downloads | Not run |

Not measured: memory, CPU while idle, a page that animates, a long-lived process. The first
screenshot of a page is taken after the load event, so pages that finish drawing later
(GitHub's placeholders) are caught early; the stream design in §3.3 does not have that
limitation.

## `orochi-servo` run through its protocol (§9 items 3, 5, 6, 7)

What was run: `browser-servo/` as built by `cargo build --release` (binary 125 MB, release
build 1 m 45 s cold and about 1 m 10 s after a source change), driven over the §3.2 protocol by
`browser-servo/tools/drive.py` against pages served from `127.0.0.1` by the script itself.
`SoftwareRenderingContext`, JPEG quality 70, 1024×768 CSS pixels, this Mac. Numbers are single
runs unless a list is given; nothing here is a benchmark.

| Item | Result |
|---|---|
| Protocol on a pipe | `ready`, `url`, `title`, `history`, `loading`, `dialog`, `dialog_closed`, `refused`, `frame` all arrived and were parsed; **0** non-protocol lines on stdout (fd 1 is pointed at stderr and the protocol runs on a duplicate) |
| Click, keys, `text` into `<input>` | Worked: `hello`, `abc` reached the page. A click on a button opened an `alert()` (`dialog` with the page's message), `dialog accept` closed it and the page continued (4 of 4 runs after fixing the harness) |
| Wheel | `mouseWheel deltaY=300` scrolled a tall page to `scrollY = 300` |
| Click → next frame | A page flips a box's colour on `mousedown`; time from sending the click to the next `frame` event: `[43, 52, 27, 99, 28, 71, 10, 106, 68, 31]` ms, median 52 (an earlier run of 10: median 35). Includes the 50 ms frame throttle |
| Frame rate while a `requestAnimationFrame` page animates | **18.7 fps at ratio 1** (1024×768, mean JPEG 23.6 KB) and **18.0 fps at ratio 2** (2048×1536, mean JPEG 87.0 KB); the cap is the 50 ms throttle, so this is a floor on what the render path can do, not its limit |
| Memory | RSS 87 MB (ratio 1) and 130 MB (ratio 2) after 5 s of animation, one process, one tab |
| Idle | A static page: 2 frames in 10 s, CPU time 0.13 s → 0.14 s over the 10 s |
| `resize` with `ratio: 2` | Frames are 2048×1536 for a 1024×768 CSS viewport; click coordinates stay in CSS pixels |
| **IME, `<input>`** | **Works.** `ime` `に` → `にほん` → commit `日本` delivers `compositionstart`, `compositionupdate`×3 and `compositionend` to the page and the field's value ends as `日本`. Servo's `End` keeps what the last update left, so a commit is sent as an `Update` with the final text and then `End` (sending only `End` with the new text left `にほん` in the field) |
| **IME and typing, `contenteditable`** | **Does not work in Servo 0.8.0.** The div takes focus (`document.activeElement` is it, `isContentEditable` is true) and composition events fire, but neither IME text nor ordinary key events insert anything: `textContent` stays empty. Rich-text editors that use it will not work in the tab |
| Cookies and storage across processes | **Persist** with `--profile <dir>` (`Opts::config_dir`): a cookie set with `Max-Age` by the server was sent back by the second process (`Cookie: sid=abc123`), and a `localStorage` counter went from 1 to 2. Files written: `cookie_jar.json`, `localstorage.json`, `clientstorage/`, `webstorage/`, `cachestorage/`, `auth_cache.json`, `hsts_list.json`. They are written when the process **exits through `shutdown` or stdin EOF**; a `kill -9` was not tested and may lose the session |
| Exit | `shutdown` returns in 0.01–0.04 s and stdin EOF in 0.07 s, both with status 0, also while a page animates |
| Policy | `navigate` to `file:///etc/hosts`, `javascript:`, `data:` and `about:config` each answer `refused`; a click on an in-page link to `file:///etc/hosts` is refused by `request_navigation` and the tab stays where it was |
| Cannot create the rendering context | Exits 1 with one line on stderr (not provoked here; the path is a `process::exit(1)` after an `Err`) |

Not done: touch and pinch input; `<select>`, colour and file pickers and the context menu (the
engine drops each, which answers it with its default, and logs one line to stderr);
downloads; popups (`popup` is emitted with an empty `url`, because `CreateNewWebViewRequest`
exposes no address); `confirm()` and `prompt()` dialogs (only `alert()` was run); `back`,
`forward`, `reload`, `stop`, `zoom`, `show` and `hide` (implemented, not exercised); several
tabs in one process; a long-lived session (memory growth, hours); the user's own login site;
`kill -9` of the broker. As in the first spike, some SVG icons draw as black squares
(`GLD_TEXTURE_INDEX_2D is unloadable` in the log).
