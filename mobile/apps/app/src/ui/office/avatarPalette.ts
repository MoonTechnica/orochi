/**
 * The only place the app draws in colour: the Office's people and the provider's badge. The app
 * around them (rings, bubbles, buttons, text) is black, white and grey, and red there keeps
 * meaning "error": a person is always inside a solid white (light: black) rim and the badge is a
 * separate disc, so a terracotta shirt or an auburn head of hair is a picture in a frame, never
 * a state. `apps/app/test/theme.test.ts` permits colour literals in this file and `theme.ts` only.
 *
 * The palette of a product team page: earth and pastel in between, low to mid saturation, flat
 * (no gradients, no shadows), bright enough to hold on a near-black ground. The same in light and
 * dark, since a person sits on a ground of their own. The indexes come from `@orochi/core`.
 */

export const GROUNDS = ['#a9c4a3', '#9cc3dd', '#b7acd9', '#e3c06b', '#eba596', '#7f9cc4'] as const;
export const HAIR_COLORS = [
  '#2b2630',
  '#5a3d2e',
  '#a8683f',
  '#d9b36b',
  '#9a98a3',
  '#3f5f8a',
] as const;
export const SKINS = ['#f4d6bf', '#e8bd9a', '#d29b73', '#b87a52', '#8d5a3a', '#5e3b28'] as const;
export const OUTFITS = [
  '#e07a5f',
  '#3d405b',
  '#81b29a',
  '#f2cc8f',
  '#8fb8de',
  '#b8a9d9',
  '#f4f1de',
  '#5f7f8c',
] as const;

/** Features: eyes, brows and mouth are one dark ink; small things on the head and shoulders. */
export const INK = '#26242c';
export const WHITE = '#ffffff';
export const GEAR = '#3b3a45';
export const CAP = '#4d6a9c';
export const PIN = '#e0b84a';
export const SCARF = '#d9826a';
/** The darker half of a lapel: a flat shape over the shirt, not a shadow. */
export const LAPEL = 'rgba(0,0,0,0.16)';

/** The vendor's own colour behind the letter, for a provider with no `agent_icons` image. */
const VENDOR: Record<string, string> = {
  anthropic: '#c96442',
  openai: '#10a37f',
  google: '#4285f4',
  opencode: '#7c5cff',
  qwen: '#615ced',
  deepseek: '#4d6bfe',
  meta: '#0467df',
  mistral: '#fa520f',
  xai: '#111111',
  nvidia: '#76b900',
};
export const vendorColor = (provider: string | null | undefined) =>
  VENDOR[provider ?? ''] ?? '#6b7280';
export const BADGE_LETTER = '#ffffff';
/** The ground a vendor's image mark sits on, as on the desktop (`.badge[data-icon="vendor"]`). */
export const BADGE_IMAGE_GROUND = '#ffffff';
