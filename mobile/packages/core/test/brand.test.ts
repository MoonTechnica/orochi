import { describe, expect, it } from 'vitest';
import { brandOf } from '../src';

describe('brandOf', () => {
  it('uses the provider where it is a maker', () => {
    expect(brandOf('anthropic', 'qwen3')).toBe('anthropic');
  });
  it('follows the model for an agent in front of many makers', () => {
    expect(brandOf('opencode', 'qwen3.8-27b')).toBe('qwen');
    expect(brandOf('opencode', 'deepseek-v4')).toBe('deepseek');
    expect(brandOf('opencode', 'llama-4')).toBe('meta');
    expect(brandOf('opencode', 'devstral-small')).toBe('mistral');
    expect(brandOf('opencode', 'grok-5')).toBe('xai');
    expect(brandOf('opencode', 'gpt-5.6')).toBe('openai');
  });
  it('is null when nothing is known', () => {
    expect(brandOf('opencode', 'my-own-model')).toBeNull();
    expect(brandOf(null, null)).toBeNull();
  });
});
