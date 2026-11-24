export interface Colors {
  bg: string;
  card: string;
  raised: string;
  text: string;
  sub: string;
  line: string;
  /** The main action, and what waits for you: white on black in dark, black on paper in light. */
  primary: string;
  onPrimary: string;
  /** What waits for you: the same inversion, with a thick border and a pulsing halo. */
  attention: string;
  /** What is alive (a working head, a working ring). */
  live: string;
  /** Error, failure and destructive actions (delete, remove, stop): the only red with meaning. */
  danger: string;
  /** The red as text and icons on the surfaces (danger itself is a fill; red text on black needs lifting). */
  dangerInk: string;
  onDanger: string;
  /** The icon's eye red for a few pixels of brand, never a fill or text. Unused on purpose: a red spot on a screen reads as an error (the logo image keeps its red eyes). */
  brandAccent: string;
  muted: string;
  mine: string;
  code: string;
  mark: string;
  connectionOnline: string;
  connectionPending: string;
}

/**
 * The app's own palette, taken from its icon: a black tile, a white eight-headed serpent and red
 * eyes (the eye pixels of `assets/icon/source/source.png` are about #d81000). Black, white and
 * greys carry everything, **inverted** for the main action and for what waits for you (white fill
 * on black, black fill on paper). Red means exactly one thing: error, failure or a destructive
 * action (`danger`, `dangerInk`), plus `brandAccent`, a few pixels of the same red as the icon's
 * eyes. Not the desktop window's amber (desktop/src/app.css) on purpose.
 */
export const palette: { light: Colors; dark: Colors } = {
  light: {
    bg: '#f4f4f2',
    card: '#fbfbfa',
    raised: '#e9e9e6',
    text: '#121212',
    sub: '#555557',
    line: '#cfcfca',
    primary: '#121212',
    onPrimary: '#ffffff',
    attention: '#121212',
    live: '#121212',
    danger: '#e0160e',
    dangerInk: '#b81209',
    onDanger: '#ffffff',
    brandAccent: '#e0160e',
    muted: '#8a8a8c',
    mine: '#e1e1dd',
    code: '#e9e9e6',
    mark: '#d9d9d4',
    connectionOnline: '#1f623e',
    connectionPending: '#755018',
  },
  dark: {
    bg: '#121212',
    card: '#1a1a1b',
    raised: '#222223',
    text: '#f5f5f5',
    sub: '#9b9b9d',
    line: '#2f2f31',
    primary: '#f5f5f5',
    onPrimary: '#121212',
    attention: '#f5f5f5',
    live: '#f5f5f5',
    danger: '#e0160e',
    dangerInk: '#ff6b61',
    onDanger: '#ffffff',
    brandAccent: '#e0160e',
    muted: '#5f5f62',
    mine: '#262627',
    code: '#222223',
    mark: '#2e2e30',
    connectionOnline: '#79d7a6',
    connectionPending: '#f4c778',
  },
};

export type Tone = 'live' | 'ask' | 'ok' | 'warn' | 'danger' | 'muted';

/** A status tone as a fill or mark. Only a failure is red. */
export const toneFill = (c: Colors, tone: Tone) =>
  ({
    live: c.live,
    ok: c.live,
    ask: c.attention,
    warn: c.attention,
    danger: c.danger,
    muted: c.muted,
  })[tone];

/** The same as text. */
export const toneInk = (c: Colors, tone: Tone) =>
  ({
    live: c.text,
    ok: c.text,
    ask: c.text,
    warn: c.text,
    danger: c.dangerInk,
    muted: c.sub,
  })[tone];

/** Tight corners: a card is barely rounded, a button a rounded rectangle. */
export const radius = { chip: 6, tile: 12, card: 12, pill: 10 };
export const space = { xs: 4, sm: 8, md: 12, lg: 16, xl: 24, gutter: 18 };
export const type = { caption: 12, body: 15, title: 17, headline: 24, display: 34 };

/** Headings are heavy and set close. */
export const tight = { fontWeight: '800' as const, letterSpacing: -0.6 };

export const hit = { minHeight: 44, minWidth: 44 };
export const mono = 'Menlo';
