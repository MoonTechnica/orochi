/** The makers whose mark the app draws. */
export const BRANDS = [
  'anthropic',
  'openai',
  'google',
  'qwen',
  'deepseek',
  'meta',
  'mistral',
  'xai',
  'nvidia',
] as const;
export type Brand = (typeof BRANDS)[number];

// opencode and its kind are one agent in front of many makers: the mark follows the model.
const MODEL_BRANDS: [RegExp, Brand][] = [
  [/qwen|qwq/i, 'qwen'],
  [/claude/i, 'anthropic'],
  [/gpt|\bo[134]\b/i, 'openai'],
  [/gemini|gemma/i, 'google'],
  [/deepseek/i, 'deepseek'],
  [/llama/i, 'meta'],
  [/mistral|mixtral|codestral|devstral|magistral/i, 'mistral'],
  [/grok/i, 'xai'],
  [/nemotron/i, 'nvidia'],
];

/** Whose mark an agent wears: its provider's where that is a maker, else its model's. Same as the desktop's `brandOf`. */
export function brandOf(provider?: string | null, model?: string | null): Brand | null {
  if ((BRANDS as readonly string[]).includes(provider ?? '')) return provider as Brand;
  return MODEL_BRANDS.find(([pattern]) => pattern.test(model ?? ''))?.[1] ?? null;
}
