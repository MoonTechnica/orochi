export interface PairingLink {
  host: string;
  port: number;
  code: string;
  name: string;
  /** The desktop's machine id; absent in a link from an older desktop. */
  id?: string;
}

/** Parses `orochi://pair?host=&port=&code=&name=`; returns null for anything else. */
export function parsePairingLink(input: string): PairingLink | null {
  const text = input.trim();
  const match = /^orochi:\/\/pair\?(.*)$/i.exec(text);
  if (!match) return null;
  const params = new URLSearchParams(match[1]);
  const host = params.get('host');
  const port = Number(params.get('port'));
  const code = params.get('code');
  const rawId = params.get('id');
  const id = rawId && /^[A-Za-z0-9]{8,64}$/.test(rawId) ? rawId : undefined;
  if (!host || !code || !Number.isInteger(port) || port < 1 || port > 65535) return null;
  if (!/^[A-Za-z0-9._:-]+$/.test(host) || /\s/.test(code)) return null;
  return { host, port, code, name: params.get('name') ?? host, ...(id ? { id } : {}) };
}

export const baseUrl = (host: string, port: number) =>
  `http://${host.includes(':') && !host.startsWith('[') ? `[${host}]` : host}:${port}`;
