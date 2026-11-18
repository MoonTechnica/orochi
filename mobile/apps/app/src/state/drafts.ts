/** Unsent text per composer, kept while the app runs so leaving a thread and coming back keeps it. */
const drafts = new Map<string, string>();

export const getDraft = (key: string | undefined): string => (key ? (drafts.get(key) ?? '') : '');

export function setDraft(key: string | undefined, text: string) {
  if (!key) return;
  if (text) drafts.set(key, text);
  else drafts.delete(key);
}
