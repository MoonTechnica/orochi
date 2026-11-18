import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { resolve, dirname } from 'node:path';

const here = dirname(fileURLToPath(import.meta.url));
export const fixture = (name: string): any =>
  JSON.parse(readFileSync(resolve(here, '../../../../desktop/tests', `${name}.json`), 'utf8'));
