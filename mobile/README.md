# Orochi mobile

A phone app that reads and drives the conversations the Orochi desktop window shows, over
Tailscale. Design: `docs/mobile-remote-design.md`. The Office is on the phone too (a mobile layout of the
desktop's Office; Microsoft Office documents are not previewed).

Screenshots named below (`e2e/screenshots/`) are local evidence from the runs and are git-ignored;
regenerate them with `e2e/run-ios.sh` or the web build.

```
mobile/
  apps/app/            Expo SDK 58 + expo-router (screens in app/, components in src/)
  packages/protocol/   wire types (mirror src/activity.rs, desktop/src-tauri/src/view.rs), RemoteClient
  packages/core/       pure logic: timeline, team-channel folding, markdown/media links, @mentions, i18n
  tools/mock-server.mjs  a stand-in for the desktop's remote server, built from desktop/tests/*.json
  tools/mock-long-thread.mjs  the same with a 300-post team channel (for scroll measurements)
```

## Setup

```sh
cd mobile
npm install
npm run typecheck      # all workspaces
npm test               # protocol + core unit tests (vitest), core tests read desktop/tests/*.json
mise run mobile        # the same two, from the repository root
```

## Try it without the desktop

```sh
node tools/mock-server.mjs          # prints a pairing link (--office: the desktop's recorded Office data, replayed; --office-large: 120 people in 36 rooms; --quiet: an empty office); code MOCK-1234, reusable (--once to burn it)
npm run start -w @orochi/app        # then press w for the web build, or scan with Expo Go
```

On the pairing screen paste the printed `orochi://pair?...` link. The mock sends CORS headers only
so the web build can be tried in a browser; the real server does not need them on a phone.
For a phone, start the mock with `--host <your LAN or tailnet address>`.

## Run on a phone

1. On the desktop: Settings → Orochi mobile app → enable, then "Pair a phone" shows a QR code (valid 5 min,
   single use). The desktop binds only to its Tailscale address, so the phone must be on the same
   tailnet (Tailscale app installed and connected).
2. Install the app: `npm run start -w @orochi/app` and open it in **Expo Go**, or build a
   **development build** (`npx expo run:ios` / `run:android` in `apps/app`, or EAS).
3. Scan the QR code (or paste the link). The token is stored in the keychain/keystore
   (`expo-secure-store`).

Expo Go vs dev build: camera scanning, secure store, audio/video and sharing work in Expo Go.
Plain-HTTP traffic to a tailnet address is allowed by `app.json` (`NSAllowsArbitraryLoads` on iOS,
`usesCleartextTraffic` on Android); Expo Go's own configuration decides what it permits, so if
cleartext is blocked there, use a development build, which carries this app's own settings.
Saving to the photo library needs the `expo-media-library` permission strings in `app.json`
and is most reliable in a development build.

## Several desktops

One phone can be paired with several desktops (a work Mac and a home Mac, each running
`orochi-desktop` or `orochi-remote`); several phones on one desktop already worked (a token per
device). Each desktop has a stable random id (`<data>/remote/machine.json`), returned by `/health`,
`/me`, `/pair` and carried in the pairing link as `&id=`. Older links and servers without one still
work (matched by host:port).

- Storage (`packages/core/src/machines.ts`, pure): a list of machines; tokens in one secure-store
  slot each (`orochi.token.<key>`), the rest in one record (`orochi.machines`). The earlier
  single-machine record (`orochi.session`) is migrated on first launch without a re-pair.
- Pairing a known id (or, without one, a known host:port) updates that machine instead of adding
  one. A link for another desktop adds it and offers "Switch to ..." / "Stay here". "Pair again"
  on a machine replaces that machine whatever id now answers.
- Scoping: everything below the session (`DataProvider`, its WebSocket, thread caches) is keyed by
  the active machine and rebuilt on a switch; media/file/event URLs come from that machine's
  `RemoteClient`. A 401 marks only that machine "needs pairing again".
- Home (first tab, `app/(tabs)/index.tsx`, pieces in `src/ui/home/`): a "serpent line" across the
  top whose eight slots are the live conversations across all desktops (a white head with a pulsing white halo = waits
  for you, a smaller white head with a flowing white line = working, grey = background; tap one to open it) with a
  one-sentence status under it; open questions and @you mentions as action cards (Allow once /
  Deny straight from Home through the `answer` RPC; Reply opens the conversation with the composer
  focused and `@sender ` typed; swipe or Dismiss hides **that one mention card** on the phone and
  calls `see_mentions` (which is per thread on the desktop) only when no other mention of that
  thread is left here, so nothing is marked read that you did not dismiss or open; two cards then
  "N more waiting"). The cards come first, above the Office card, so the first card's buttons are
  on the first screen of a small phone; desktops as a horizontal strip of nests (state colour, six
  bars of **conversations touched** in the last hour, captioned and labelled so, "add" tile; an
  offline or needs-re-pair nest shows its reason, Retry or "Re-pair" on tap); recent work as a trail: projects are branches of a rail,
  conversations nodes on it. Without any desktop it is the three-step guide. Desktop names are
  shown only with more than one desktop. Reduced motion stops the animation.
- Look (2026-11-17, restyled after the icon, then red narrowed to errors): one theme
  (`src/ui/theme.ts`) in the icon's world, a black tile and a white eight-headed serpent. Near-black
  `#121212` with neutral `#1a1a1b` panels in dark (the main mode), paper `#f4f4f2` with black text in
  light, mid-grey secondary text, hairline greys. **The main action and what waits for you are the
  inversion**, not a colour: a white fill with black text in dark, a black fill with white text in
  light (`primary`/`onPrimary`, `attention`), and a waiting card also gets a thick white (light:
  black) border and the serpent's waiting head a pulsing halo; badges are the same inverted chip.
  **Red means error and nothing else** (`danger`, `dangerInk`, `onDanger`, `#e0160e`, the icon's eye
  red; it is used for failures, an unreachable desktop, failed checks and destructive buttons:
  Remove, Stop). A desktop that must pair again is not an error: white border and an inverted
  "Re-pair" button. `brandAccent` (the same red for a few pixels of brand) exists but is **unused**
  on purpose, because a red dot in a serpent head read as an error. No amber, yellow or green, no
  shadows, 1 px hard borders, corners of 6-12, heavy tightly set headings and small uppercase labels.
  **Colour appears in exactly one place**: the Office's people (minimal flat vector people in the style of a product team page, `react-native-svg`: a round head, a plain mass of hair or headwear without outlines, a shirt, two dots for eyes and one thin line for a mouth, no gradients and no shadows, earthy-pastel low-saturation colours and varied skin tones; a name picks one of ten hair and headwear shapes (half-round, sleek, bun, bob, curly, ponytail, beanie, hijab, turban, buzz), colours, shirt and ground, and a quiet role mark (a blazer with a collar and a pin for the lead, headphones or a cap for an implementer, thin round glasses for a reviewer, nothing on the small sub-agent drawn beside a seat; a scarf, collar or earring for the others), so the same agent is the same face everywhere; the state picks the face: eyes cast down and a blink while working, raised brows and a small round mouth while asking, a deeper smile when done, short lines for closed eyes when idle, a moving mouth while talking, still under reduced motion; the selection logic is pure and tested in `packages/core/src/avatar.ts`; every look is on one page, the address `/avatars`, drawn at Home's Office card) and the provider badge on them (the vendor's colour, or `agent_icons`' image on white). Those colours live in `src/ui/office/avatarPalette.ts`, the one file the palette test allows colour literals in besides `theme.ts`; the test also fixes who may read it (`Avatar`, `Character`, `parts/Accessory`) and that every avatar is drawn inside a solid rim in the text colour, so a person whose hue is red-ish is a picture in a frame and not a state. The ring (white pulse while working, thick white while asking), bubbles, names and everything else around them stay black, white and grey. `apps/app/test/theme.test.ts` checks the
  palette is greys plus red, that primary and attention are the inversion, that the red tokens are
  read only in an allowlist of files (errors and destructive actions), that no screen has a hex
  colour, and WCAG AA (4.5:1) for every text colour on every surface. **This is intentionally not the
  desktop's palette** (the window keeps its amber, `desktop/src/app.css`). A floating bar with a thin
  border and the inverted + in the middle (`src/ui/TabBar.tsx`), the logo (`src/ui/Logo.tsx`, a PNG).
  Other screens only inherit colours, the header title and the big tab titles. The bar sits
  below the screens (not over them); `useTabBarHeight()` has its measured height, and the repair
  cover below uses it.
- Icons: the art is the desktop's own icon (white eight-headed serpent, red eyes, black tile):
  `apps/app/assets/icon/source/source.png` is a copy of `desktop/src-tauri/icons/orochi/source.png`
  and is never redrawn. `node tools/make-icons.mjs` (needs `npm install`; `@resvg/resvg-js` and
  `pngjs` pinned) lifts the emblem off the tile and writes `apps/app/assets/icon/*.png`: iOS
  light/dark/tinted (full-bleed, no alpha), Android adaptive foreground/background/monochrome,
  splash, favicon, the 96px notification icon and `logo.png` (the in-app logo, the emblem on its own
  black tile, used by Home, the guide and Settings). It also writes `e2e/screenshots/icon-sheet.png`.
  Changing icons or splash needs `expo prebuild` and a new dev build.
- Switcher: the machine name with a status dot at the top of Conversations opens a sheet to
  switch, rename (a local label), remove, pair again, add, and "Needs you on all desktops".
  Settings has the same list. Two desktops with one name are told apart everywhere (switcher,
  Settings, Home, Office chips, alerts) as `name (host:port)`.
- The conversation's "..." opens a right-hand drawer (`src/ui/Drawer.tsx`, reusable: 80% width,
  dimmed backdrop, close by tap, swipe right or back; a strip on the right edge opens it).
- A desktop that needs pairing again covers the screens with a "Pair again" panel (switcher, Pair again
  and "Remove this desktop" on it) but **never the tab bar**, and not Settings: the other desktops
  stay reachable.
- Other desktops are **polled every 20 s while the app is in the foreground** (sidebar, prompts,
  mentions, seats, tasks); only the active desktop has a live WebSocket. Notifications name the
  desktop when more than one is paired.

## Pairing flow

`orochi://pair?host=&port=&code=&name=&id=` -> `POST /pair {code, device, device_id}` -> `{token, name}`.
`device_id` is a random id made once per install (`orochi.install` in the secure store); pairing again
with the same one replaces that phone's old row on the desktop (without it nothing is guessed from
the name). The desktop's name is editable in Settings → Orochi mobile app (default: host name without domain). The token
is sent as `Authorization: Bearer` on `/me` and `/rpc/<command>`, and as `?token=` on `/media`,
`/file` and the `/events` WebSocket. A 401 anywhere unpairs the app.

## What is implemented

- Pairing: QR scan, paste a link, manual host/port/code form, and the `orochi://pair?...` deep link
  (a tapped link or `xcrun simctl openurl booted '<link>'` pairs at once, also re-pairing a paired
  app). A 401 (token revoked on the desktop) marks that desktop as needing pairing again (Home card,
  switcher, a panel on its tabs); the others are untouched. Remove a desktop, language en/ja, theme follows the system.
- Conversation list: pinned, projects, status marks, needs-you/unread, an Archived view, search,
  pull to refresh, new thread per project, live updates from WebSocket `changed` events (with
  polling fallback). Long-press a row or tap its `...` for rename, pin, archive/unarchive, route
  (agent/model from `insights` + `agents`, as the desktop's route menu) and effort.
  No swipe actions (long-press and the button instead). Project rename/pin/hide are not on the
  server whitelist and are not offered.
- Conversation: messages grouped by speaker, Markdown, folded tool calls/notes, checks, assignment
  posts with member bars, thread panel screen, reports lines, approval prompts, `@` completion, the Office's characters as faces (see "Faces in the conversation"),
  send/queue/interrupt, changes list with diff viewer. Turn footers ("Worked for ...") now stay
  under their own turn when turns overlap (`packages/core` regroups entries by turn; regression test).
- Files (`...` menu): directory tree with git status marks, find by name or content, text/code
  viewer, image/video/audio, changed files per scope (`unstaged`/`staged`/`branch`) with the
  diff from `tree_patch`, and review comments: tap a line of that diff, write a note, "Send
  review" posts them with `comment` (the same message the desktop sends).
- Git: status counts, worktrees, branches, commits with refs; a commit opens its diff (`git_show`).
- Work: helpers' worktrees, trials (files/overlap/conflicts/checks), the plan board, and what is
  waiting on you. Approvals are answered in the conversation as prompts.
- Office (`app/office.tsx`, pieces in `src/ui/office/`, logic in `packages/core/src/office.ts`): the desktop's home
  screen for a phone. Home has an "Office" card (counts, a few faces) that opens it. Projects are
  collapsible floors, conversations rooms stacked down the page (the one that needs you first, as
  on the desktop), each room a rounded panel: title, status, its team as a flowing three-column grid
  of the desktop's own characters (bust drawn in one hue from the agent's name, `react-native-svg`),
  round avatar with a ring (solid and pulsing while working, dashed while idle, a thick white (light: black) ring while
  asking), provider badge (the desktop's `agent_icons` image when it has one, else the letter), a
  bubble with what it does or says, sub-agents beside their seat, an envelope on the one a message
  is for. Counters across the top as the desktop's. Tap a room: that conversation; tap an avatar: a
  sheet (agent, model, what it does, sub-agents, Open thread / Open the question, Stop this seat
  through `stop_seat`). With several desktops a chip row picks which one (the others are polled
  every 20 s as everywhere). Data is exactly the desktop's: `sidebar`, `live_seats`, `live_tasks`,
  `chatter` (read on every change, at most every 3 s, and every 10 s), `agent_icons`. **Simplified on
  purpose:** the desktop's wires and flying envelopes across rooms are not drawn; a room shows its
  last three messages as lines (marked ↗ / ↘ with the other room's title when they cross rooms),
  an envelope hops once on a fresh one, the speaker's mouth moves, and an unopened message marks
  its addressee until opened. Arrival, leaving (wave and fade), a finished room staying a minute as
  done, and a deleted room closing are the desktop's `stepOffice` rules; animation uses Reanimated
  and stops with reduced motion. `reported` (spoke and then stopped) and `mentioned` (an
  unopened message for it) are states of this app, derived from `chatter`; the desktop draws neither.
- Office at scale (2026-11-18; `packages/core/src/officeScale.ts`, tested in `test/officeScale.test.ts`; screen `app/office.tsx`):
  the page is one `FlatList` of floor headers and room rows (`officeItems`), so only rooms near the viewport are
  mounted (`windowSize` 3); pull to refresh, floor folding, the machine chips, the Strip and the filter chips are the
  list header. A room draws at most `ROOM_CAP` = 6 people (asking, then working, then mentioned/reported, then
  idle/leaving; the drawn ones keep the room's own order) and a "+N" tile with a dot summary that opens a sheet listing
  everyone (avatar 32, name, agent · model, state, doing); a person behind it opens the same seat sheet, Stop included.
  A floor of more than `COMPACT_FLOOR_ROOMS` = 8 rooms, or a room of more than `COMPACT_ROOM_PEOPLE` = 12, is a one-line
  row (state icon, title, up to `COMPACT_FACES` = 5 overlapping faces and "+N", a "Needs you" marker) that opens in place;
  one picked room per floor, and rooms that ask are open until closed. Motion (`useOfficeMotion`, `src/ui/office/motion.tsx`)
  runs only for rooms the list reports as viewable, only for drawn people, never when more than
  `MOTION_PEOPLE_LIMIT` = 24 seats are on screen, under reduced motion, or with the app in the background. Chips All /
  Needs you / Working (room counts) under the Strip; the Strip's "at work", "need you" and "rooms open" tiles are the
  same filters; the choice lives for the app session (`src/state/officeView.ts`); each filter has an empty text.
  `tools/mock-server.mjs --office-large` (generator `tools/office-large.mjs`, fixed seed) serves 120 people in 36 rooms
  over 5 projects with sub-agents and messages. Dev builds record `globalThis.__officeTiming` (first paint, render ms).
- Activity: live seats with sub-agent tasks as a tree, answerable prompts, mentions, chatter, route stats.
- Media: inline images, full-screen viewer with swipe, pinch zoom (about the point between the fingers),
  double tap to zoom in on the tapped point and again to fit (maths in `packages/core/src/zoom.ts`,
  tested), save to library, share,
  video (`expo-video`), audio (`expo-audio`), gallery grid.
- Notifications (Settings toggle): local notifications raised by the app itself from what it reads
  over the tailnet: a new question waiting, an `@you` mention, a turn that stopped running; badge =
  questions + mentions. No push service (APNs/FCM/Expo push) is used, so no content leaves the
  tailnet. **Limit, stated plainly:** iOS suspends a backgrounded app within seconds and drops the
  WebSocket, and nothing runs when the app was killed, so nothing is delivered then. Real
  background delivery would need a push service (content through Apple) or a background-fetch
  task (iOS decides when, roughly every 15 minutes or never); neither is built.
- Not remote, on purpose: memory settings, terminal, settings, sandbox, anything off the server
  whitelist.

## iOS end-to-end

```sh
cd mobile/apps/app && npx expo run:ios --device <udid> --no-bundler   # once: installs the dev build
mobile/e2e/run-ios.sh        # needs maestro and a JDK (JAVA_HOME or /opt/homebrew/opt/openjdk)
```

It builds `orochi-remote`, starts two with demo data, starts Metro, boots the simulator (`SIM_UDID`
picks one; `KEEP=1` leaves everything running), pairs by `simctl openurl` between the flows and
copies the screenshots. `simctl io screenshot` hangs when it writes to an external volume, which is
why maestro's own screenshots are used.

## Checks

`mise run mobile` (typecheck + tests; needs `npm install` in `mobile/` once). It is **not** in
`mise run check`: that chain must work on machines with no Node install, so it stays a separate task.

## Verified here, and not

iOS simulator (2026-11-17, "iPhone 17", iOS 27.2, Xcode 27.2, a development build; `e2e/run-ios.sh`
runs five maestro flows in `e2e/flows/` against two real `orochi-remote --demo-data` instances on
127.0.0.1:47821/47822 and Metro; 51 screenshots `e2e/screenshots/ios-NN-*.png`):

- Fresh install: the empty guide (ios-01) and the scan screen on a device with no camera (ios-02: the
  preview area is blank, a line says to paste the link or enter the details, both stay usable).
- Pairing by deep link (`xcrun simctl openurl`, cold start and while running; iOS may first ask
  "Open in Orochi?"), Home dashboard, conversation list, plain-HTTP media from 127.0.0.1 (images,
  video, audio over the cleartext setting), full-screen viewer with swipe between items.
- Audio: the clock advances (0:00, 0:02, 0:06 of 0:07) and the player finishes. Video: the frame
  changes after play, so it is decoded and played. Whether sound comes out of the speaker cannot be
  seen here. The demo clips are H.264 + AAC in MP4/M4A (the only formats the demo serves); other
  codecs (WebM, Opus, HEVC variants) were not tried.
- Right-hand drawer: strip drag from the edge opens it, a swipe closes it, the backdrop closes it.
- Files (text, folder, image, changed), Git (status, commit diff), team channel, thread panel, the
  keyboard under the composer and the thread reply field (the composer rises above it), approval
  answered (the buttons go away; the demo seat itself does not resume), a queued message, Activity
  and Usage, row menu, rename, pin, archive and the Archived list.
- Two desktops: the "Updated/Added" offer after a second link, Home with both, the switcher, rename
  and switch, a revoked token (the app sees it within the poll interval and shows the panel, Home
  card and Settings entry in orange), pairing again by link replaces the same desktop, removing both
  desktops (Alert confirm) ends on the pair screen.
- The notification permission prompt appears from the Settings toggle and the toggle turns on.
- `npm run check` (typecheck, 50 core tests, 11 protocol tests), `expo export --platform ios`;
  web build against the real server through `e2e/cors-proxy.mjs` as before (screenshots
  `e2e/screenshots/web-*.png`, `home-*`, `multi-*`, `drawer-*`).

Office (2026-11-17): unit tests of the rules against `desktop/tests/office.json` (`packages/core/test/office.test.ts`);
web build with agent-browser against `tools/mock-server.mjs --office` and a real `orochi-remote --demo-data`
(screenshots `e2e/screenshots/office-NN-*.png`: Home entry, rooms, avatar sheet, a stopped seat, light, Japanese,
quiet office, two desktops, a room opening its thread); iPhone 17 simulator through `ONLY=office KEEP=1 e2e/run-ios.sh`
(flows `7-office.yaml`, `7b-office-ja-dark.yaml`, `ios-office-NN-*.png`: Home card, rooms, sheet, stop seat, fold a floor, room opens the
conversation, Japanese dark). Not verified: VoiceOver reading order, reduced motion on a device, the vendor icon image
(`agent_icons` is empty on the mock and the demo), a real desktop with real chatter, Android.

Bugs this run found and fixed (all in `apps/app`): the drawer never closed by swiping on iOS
(its move events never reached the panel from inside the scroll view; the panel now claims the touch at its start and a swipe to the right on release closes it); sheets
(`Sheet`, new thread, desktop switcher) hid their buttons from the accessibility tree because a
backdrop `Pressable` wrapped them, and from the second opening on the whole sheet was invisible to
VoiceOver/XCUITest (now sibling backdrop + `accessibilityViewIsModal`); the first tap on "Save" of a
desktop rename was swallowed by the keyboard dismissal inside a `ScrollView` and the field sat under
the keyboard (inline prompt, `keyboardShouldPersistTaps`, keyboard avoidance); the scan screen gave
no hint where there is no camera (a line pointing to the paste form is shown; `onMountError` adds a
second message, which the simulator never triggered); audio of under a second read
"0:00 / 0:00" (duration rounds up); the dev-only "Open debugger to view warnings" toast covered the
tab bar (one harmless native-driver warning is ignored).

Second pass (2026-11-17, iPhone 17, `e2e/run-ios.sh`, every flow passes): Home with the needs-you
cards first (the first card's buttons and the tab bar are on the first screen, and the list
scrolled to its end shows the last row fully above the bar: `ios-home4-01..04`), Reply focusing the
composer with `@bob ` typed (`ios-home4-03`), a revoked desktop keeping the tab bar with Pair again,
Remove and every tab (`ios-41b`), two desktops of one name told apart by address (flow 3), and a
**double tap on the image viewer visibly zooming in and out** (`ios-08b/08c`, the wrapper's
accessibility label reads "Image, zoomed" / "fit to screen"). The first version did not zoom on
the simulator: the second tap of maestro's double tap comes after 300 ms, so the window is now 450
ms. Pinch is covered by the gesture-maths unit tests (`packages/core/test/zoom.test.ts`) and by
reading the code (the responder claims two touches; the point between the fingers stays under
them), **not** exercised on a screen: maestro cannot pinch.

Small phone: Home at 375x667 (web build, agent-browser) shows "Allow once / Reject" above the bar
and the end of the list above it; no iPhone 17e / SE simulator run was made.

Office characters, redrawn as flat startup-style people (2026-11-17): `npm run check` (selection logic: determinism, ranges, every part used, 30+ distinct looks, a face for every state; palette rules), the web build in dark, light and Japanese (`e2e/screenshots/office-startup-NN-*.png`: every variation, states, sizes down to the Home card's 40 px, rooms, a needs-you room, the sub-agent), the whole iOS run (`ios-office-startup-NN-*.png`).

Restyle after the icon and red narrowed to errors (2026-11-17): `npm run check` (palette and red-usage tests),
`expo export`, the web build in dark, light and Japanese (Home, conversation, drawer, Office, Activity,
Settings, a failed pairing, a revoked desktop, an unreachable desktop: `e2e/screenshots/home6-NN-*.png`), and
the whole iOS run again (`ios-home6-NN-*.png`). Not judged by anyone but the author: whether it "looks like the icon"; the
splash needed no change (it was already `#121212`); the notification small icon's tint is set in
`app.json` and only a new native build shows it; Android was not looked at.

Office at scale (2026-11-18, `--office-large`: 120 people, 36 rooms, 5 projects): `npm run check` (core: ordering, cap, +N
counts, stability on a state change, compact thresholds, accordion, filters, motion rules, and the generated large office),
`expo export --platform ios`. Web (agent-browser, 390x844, `e2e/screenshots/office-scale-NN-*.png`: many rooms, compact rows,
a compact row opened, a capped room with +N, the +N sheet, a person behind it with Stop (it stops and leaves the room), both
filters, light, Japanese). Measured on the web build (dev, Chrome, one machine, small samples): mounted avatars while scrolling
the whole list 24-74 at most (the "everything at once" arrangement, remade by raising the list window, the cap and the
compaction to their maximum, mounted 120 avatars and 4,580 DOM nodes), DOM nodes at load 922 against 4,580; time from the
screen's mount to its first painted rooms 72-100 ms against about 460 ms for that arrangement, and 59-79 ms for `--office` (3 rooms,
8 avatars). Motion: of the rings in view all moved and of those out of view in the mounted window 1 of 8 did (a room taller than the
screen); with the page hidden none moved; with a 7000 px viewport (61 seats on screen, over the limit of 24) none moved. iOS
simulator (iPhone 17): the full `e2e/run-ios.sh` passes (flows 1-7b as before, new `7c-office-large.yaml`: scrolling, a compact row
opened and closed, a room's "+N" sheet, a hidden person's seat sheet, the Needs you and Working chips, the Strip tile as a
filter; `ios-office-scale-NN-*.png`). One of four full runs also failed flows 2 and 7 once (a slow start of the simulator; they
passed on the next run with no change), so the suite is somewhat flaky on a cold simulator. maestro's `scrollUntilVisible` stepped
over small targets in the long list, so 7c uses fixed swipes and tapping a row to bring its tile into view. Frame rate was
looked at only qualitatively (no dropped-frame measurement); **no physical device, no VoiceOver, no Android, no profiler numbers
on a phone.** Not seen on a screen: the +N sheet in Japanese on iOS, the filters' empty texts, a real desktop with 100+ people.

Still not verified:

- Camera QR scanning: the simulator has no camera, so decoding a QR, the permission prompt and the
  denied path (`simctl privacy` cannot revoke the camera; the code shows a "camera permission is
  needed / Allow camera or Open Settings" state and the paste form) were not exercised on screen.
- A physical device, a real tailnet (Tailscale address, MagicDNS, local-network permission prompt),
  Android, push or local-notification **delivery** (the prompt and toggle only), anything in the
  background or with the app killed, saving to the photo library and the share sheet, pinch zoom
  (maestro cannot pinch), Dynamic Type,
  landscape and iPad layouts, VoiceOver itself.
- Review comments on a real diff (the demo repository is clean), Work with real helpers, a real
  agent turn (the demo seats do not run).
- Web only: the `<button>` inside `<button>` warning earlier runs saw on the channel screen **could not
  be reproduced** in this run: the dev console of the web build (agent-browser, a real
  `orochi-remote --demo-data` and the mock) is empty of it on Home, the list, five conversations
  (team channel, media, approval, with the composer's `@` list and the "..." drawer open) and the
  Office, and a DOM scan (`button button`, `[role=button] button`, ...) finds no nested control on
  any route. The likely source, a backdrop `Pressable` wrapping a sheet's buttons, was removed in
  the earlier sheet fix. Nothing was changed for it.

Multi-desktop and Home (web build, agent-browser 390x844, two real `orochi-remote --demo-data`
instances on 127.0.0.1:47831/47832 behind two `e2e/cors-proxy.mjs`; screenshots `e2e/screenshots/home-*`,
`multi-*`, `drawer-*`): empty guide, one desktop, two desktops with different data, rename, switch,
the switcher, "Needs you on all desktops", add-desktop with the switch offer, a stopped instance
(red, others keep showing), a replaced data dir (orange, only that desktop), pair again replacing
the same machine, migration of the old single-machine record, new thread from the + sheet, the
drawer (open, backdrop close, navigation), Japanese, light and dark. Unit tests: machine store
(migration, dedup by id and by address, switching, removal, storage split, scoping) and the Home
builder in `packages/core`, URL scoping and link id in `packages/protocol`. Rust: machine id
test in `desktop/src-tauri/tests/remote.rs`.

Not verified for multi-desktop: Android, notification delivery naming the desktop, two real
tailnet hosts. (iOS simulator: the switcher, the drawer gestures and `orochi://pair?...&id=` through
the OS are verified, see above.)
Direct loads of `/conversations` on web land on Home (tab clicks work).

## Faces in the conversation (2026-11-18)

The letter-in-a-circle placeholders (`Face.tsx`) are gone. Everywhere a person or agent is drawn outside the Office it is the
Office's character, from the same pure function: `lookFor(seedOf({ role, agent }), kindOf(...))` in `packages/core/src/avatar.ts`.
`packages/core/src/people.ts` is the only new core code (`whoOf`, `whoNamed`, `lookOfWho`, `personStateOf`, `agentNamed`, tested in
`packages/core/test/people.test.ts`): the lead is recognised by the thread's own lead role, so `lead|claude` is the same face as in
the Office, and a `Voice`, a member, a mention (through `NeedItem.from`) and a seat all resolve to one `Who`.

Where they appear:

- Messages (`PostView`): 34 px, on the first message of a group only (a run of one speaker has none, as before), for the lead,
  members, `@channel` posts, a mention-bearing post and you. Text moved from a 54 to a 56 px indent (`INDENT`).
- Assignment posts and thread bars (`BarView`): the member's face (30 px) plus up to two other agents that replied under the same role.
- Thread panel (`member/[name]`): a head row with the member at 44 px and `agent · model`; its reply rows are ordinary posts.
- "reports from": stacked 18 px faces before the line; the names stay text.
- `@` completion: 30 px per row (`@channel` keeps its icon); a live member has the working expression.
- Threads strip above the conversation (26 px beside each chip), the drawer's Threads list (30 px), the working-seat line under the
  channel (20 px).
- Approval card in a conversation and in Activity: the asker at 28 px with the asking expression.
- Conversations list: the thread's agents (22 px, overlapped).
- Home: the speaker of a mention card. Activity: each live seat (30 px, working or asking), each sub-agent (the Office's
  `MiniAvatar`), the speaker of a mention.
- `/avatars` shows you, the lead, a member, a reviewer and an unplaced speaker at 34 and 22 px.
- Text only, on purpose: Activity's chatter and usage tabs, Home's approval cards (the prompt names no agent), Settings, the
  file, Git and diff screens.

**You** are not an agent look. They are one calm character from the same set (`kind: other`, never a role mark or accessory),
picked by a seed made once per install (`state/you.ts`, kept in the secure store under `orochi.you`, never sent anywhere), so it is
stable on this phone and differs between phones. Two cues tell it from an agent at a glance without a colour: a thin ring in the
text colour set off from the face by a gap in the background colour (agents have one solid rim, no gap), and no provider badge
(agents carry theirs from 28 px up). A speaker with no role and no agent (a system line) is an initial on a plain circle.

State: only where a live state exists. A member's state maps onto the Office's (`personStateOf`: working, asking, reported, idle) and
`faceOf` gives the expression; messages always use the smile. **Motion:** none in the message list; faces there are
`Character still`, which has no reanimated hooks at all (`Character.tsx` got a `still` path; the Office's own drawing is unchanged).
Motion is allowed only for `LiveFace` (a thread bar while its member works, the thread panel head), which is the Office's own
`Avatar` (pulse, bob, blink) and stops under reduced motion. `WhoFace` is `React.memo`ed on identity, size, state and name, so a
long conversation does not redraw faces; `FlatList` now keeps about seven screens mounted (`windowSize 7`) instead of 21.

Accessibility: a face is hidden from the accessibility tree where the name is written next to it (every place above); with
`decorative={false}` it is an image named for the person. i18n: no new strings.

Colour: no new colour literal anywhere. `WhoFace` imports `Character` and `ProviderBadge` (extracted from `Avatar.tsx`, drawing
unchanged) and never the palette, so the palette test's allow-lists are unchanged; `apps/app/test/chatAvatars.test.ts` adds that the
placeholder is gone, that the chat faces use no reanimated, and that the lists never import the moving avatar.

Long thread: `node tools/mock-long-thread.mjs [--turns 100] [--port N] [--code C]` runs the mock desktop with about 300 more posts in the
team channel. It does not edit `mock-server.mjs`: it reads it, splices the rows in after the channel fixture is loaded, runs the copy
(`tools/.long-thread.gen.mjs`, removed on exit) and passes every other argument on.

Measured on the web build (headless Chrome, software rendering, 390x844, 100 turns = about 300 posts, a scripted scroll of 40 px per frame
for 4 s over about 9,500 px, a second pass with faces replaced by empty boxes, windowSize 21 for both):
the faces cost 4.7k DOM nodes (7.8k against 3.1k for 238 mounted faces) and p95 frame time went from 29.5 to 31.3 ms, p50 16.6 ms
either way, worst frame about 86 ms in both (the first frame). After `windowSize 7` the same scroll mounts 44 faces at rest and 88 at the
end (not 120 and 238), 2.9k nodes, p95 20-21 ms, worst frame 38-45 ms. That is a software-rendered browser; a phone's frame rate was
not measured (see "Verified here, and not").

## Known gaps

- A phone paired by an older app version has no `device_id`; its row stays until revoked or until it
  pairs again with the new version (the next re-pair then has an id to replace by).
- Other desktops are polled, not subscribed; the combined view can lag by up to 20 s and is paused
  in the background.

- No offline cache; no background delivery (see Notifications); no project-level actions.
- Dev dependencies include `react-native-gesture-handler`/`reanimated`/`worklets` only because the
  router stack expects them; the app's own gestures use the core PanResponder.

## Threads like Slack's (2026-11-18)

Members' threads are drawn the way Slack's iOS app draws threads, **inside the existing data model**: a thread is a member's
conversation hanging from the lead's assignment post (`Said` rows with `member`/`parent`), "also send to channel" is the row's
`broadcast`, and the person can only reply in a thread that exists (`say` with `member`). There is no way to start a thread on an
arbitrary message (the server has no such operation), so a message that is not part of a member thread shows no thread affordance.
Pure logic is `packages/core/src/threads.ts` (tests: `packages/core/test/threads.test.ts`); screens are `src/ui/Threads.tsx`,
`app/thread/[id]/member/[name].tsx` (the thread) and `app/thread/[id]/threads.tsx` (the list).

- Channel: under an assignment post, a **thread footer** replaces the bordered member bar: up to three distinct repliers (20 px static
  faces, overlapping), "N replies", "Last reply 4m ago", a chevron, the whole row tappable. Unread is a dot and heavier text; a thread
  that asks you or mentioned you shows a "Needs you" chip on the second line, beside the member's work caption ("working — ...",
  "trial clean", "failed"; "done" is left out). Label for a screen reader: "Thread implementer: 6 replies, last reply 4 minutes ago,
  Needs you, unread". A reply also sent to the channel appears in the body under a "Replied to a thread" caption that opens the thread.
- Unread: kept on the phone only, per machine, conversation and member (`src/state/threadSeen.ts`, the time of the newest reply seen, in
  the secure store the app already uses; AsyncStorage was not added because it needs a new native build). Your own replies are never
  unread. Opening a thread marks it seen; `see_mentions` is still called after a reply.
- Thread screen: header "Thread" with the conversation title under it; the parent post in full; "N replies" with a hairline; the
  replies as ordinary messages (with "Today" / "Yesterday" / date dividers); the member's live work as a quiet folded line and a quiet
  "working" row at the end; a sticky composer "Reply..." with the **"Also send to the channel" checkbox**, which applies to one message
  and resets after sending. The draft is kept per thread while the app runs (`src/state/drafts.ts`). Opening lands on the first unread
  reply (or the bottom; or the top when almost everything is unread); a "New replies" pill appears when replies arrive while scrolled up.
- Threads view (full screen): cards with the repliers' faces, the parent's first line, the latest reply and its time, the unread dot and
  "Needs you"; unread or needing you first, then by latest reply. Opened from a single "Threads N" pill (with a dot when something is
  unread) under the channel header, from the drawer's "Threads (N)" entry, and from a long press's "Reply in thread". The horizontal chip
  strip and the drawer's member list are gone.
- Messages: day dividers; a tap shows the time on each message (in the face column for a message that continues a run); a long press
  opens "Copy text" and, only on a message that belongs to a member thread, "Reply in thread".
- Chosen deviations: ticking "Also send to the channel" still sends through `send` as a message to the lead (`name: text`), as before,
  since `say` has no broadcast argument; so your own reply is not itself drawn with the "Replied to a thread" caption. The header has the
  pill under it rather than a fourth header button. On iOS a long press on selectable text also raises the system "Copy" callout.
- Mock: `node tools/mock-threads.mjs [--port N] [--code C] [--drip SECONDS]` (a wrapper like `mock-long-thread.mjs`; `mock-server.mjs` is not
  edited): the channel moved to now with an earlier turn from yesterday, a reviewer thread with six replies from three people, a reply
  also sent to the channel, a third member `docs` that asks you, and optionally a reply every N seconds.
- Verified (web, agent-browser, 390x844, `e2e/screenshots/threads-NN-*.png`: footers with unread and needs-you, thread, checkbox ticked,
  after sending, Threads list, drawer entry, day dividers, long-press sheet, "New replies" pill, light, Japanese); `npm run check`;
  `expo export --platform ios`; iPhone 17 simulator, `ONLY=threads e2e/run-ios.sh` (flow `8-threads.yaml`, `ios-threads-NN-*.png`) and the
  whole `e2e/run-ios.sh`. Flow 2 now opens the demo's alice thread through its footer. **Not verified:** VoiceOver reading, a physical
  device, Android, the "New replies" pill and first-unread jump on the iOS simulator (web only), the checkbox state is read from screenshots
  (maestro cannot read it), real replies arriving from a real desktop. Metro on the external volume did not see file changes: the web
  checks needed a restart with `--clear` after each edit.
