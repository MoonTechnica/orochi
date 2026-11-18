import { describe, expect, it, vi } from 'vitest';
import { RemoteClient, RemoteError, parsePairingLink, baseUrl, api } from '../src';

describe('pairing link', () => {
  it('parses a full link', () => {
    expect(
      parsePairingLink('orochi://pair?host=100.64.1.2&port=7421&code=ABC123&name=My%20Mac'),
    ).toEqual({
      host: '100.64.1.2',
      port: 7421,
      code: 'ABC123',
      name: 'My Mac',
    });
  });
  it('rejects other schemes, bad ports and missing code', () => {
    expect(parsePairingLink('https://x')).toBeNull();
    expect(parsePairingLink('orochi://pair?host=1.2.3.4&port=0&code=a')).toBeNull();
    expect(parsePairingLink('orochi://pair?host=1.2.3.4&port=80')).toBeNull();
    expect(parsePairingLink('orochi://pair?host=a b&port=80&code=x')).toBeNull();
  });
  it('brackets an IPv6 host', () => {
    expect(baseUrl('fd7a::1', 80)).toBe('http://[fd7a::1]:80');
  });
});

const reply = (body: unknown, status = 200) =>
  new Response(typeof body === 'string' ? body : JSON.stringify(body), { status });

describe('RemoteClient', () => {
  it('sends the bearer token and JSON body to /rpc', async () => {
    const fetchMock = vi.fn(async () => reply([{ id: 'p' }]));
    const c = new RemoteClient({ base: 'http://h:1/', token: 'tok', fetch: fetchMock as any });
    const out = await api(c).sidebar();
    expect(out).toEqual([{ id: 'p' }]);
    const [url, init] = fetchMock.mock.calls[0] as any;
    expect(url).toBe('http://h:1/rpc/sidebar');
    expect(init.method).toBe('POST');
    expect(init.headers.Authorization).toBe('Bearer tok');
    expect(JSON.parse(init.body)).toEqual({ limit: 200, archived: false });
  });
  it('pair and health send no token', async () => {
    const fetchMock = vi.fn(async () => reply({ token: 't', name: 'Mac' }));
    const c = new RemoteClient({ base: 'http://h:1', token: 'old', fetch: fetchMock as any });
    await c.pair('CODE', 'iPhone');
    const init = (fetchMock.mock.calls[0] as any)[1];
    expect(init.headers.Authorization).toBeUndefined();
    expect(JSON.parse(init.body)).toEqual({ code: 'CODE', device: 'iPhone' });
  });
  it('pair sends the install id when there is one', async () => {
    const fetchMock = vi.fn(async () => reply({ token: 't', name: 'Mac' }));
    const c = new RemoteClient({ base: 'http://h:1', fetch: fetchMock as any });
    await c.pair('CODE', 'iPhone', 'abc123');
    const init = (fetchMock.mock.calls[0] as any)[1];
    expect(JSON.parse(init.body)).toEqual({ code: 'CODE', device: 'iPhone', device_id: 'abc123' });
  });
  it('turns { error } bodies into RemoteError with the status', async () => {
    const c = new RemoteClient({
      base: 'http://h:1',
      token: 'x',
      fetch: (async () => reply({ error: 'bad code' }, 401)) as any,
    });
    const error = await c.me().catch((e) => e);
    expect(error).toBeInstanceOf(RemoteError);
    expect(error.unauthorized).toBe(true);
    expect(error.message).toBe('bad code');
  });
  it('reports an unreachable machine as status 0', async () => {
    const c = new RemoteClient({
      base: 'http://h:1',
      fetch: (async () => {
        throw new Error('nope');
      }) as any,
    });
    const error = await c.health().catch((e) => e);
    expect(error.status).toBe(0);
  });
  it('builds media and file urls with the token', () => {
    const c = new RemoteClient({ base: 'http://h:1', token: 'a b' });
    expect(c.mediaUrl('t1', 'abc')).toBe('http://h:1/media/t1/abc?token=a%20b');
    expect(c.fileUrl('t1', 'a dir/x.png')).toBe(
      'http://h:1/file?thread=t1&path=a%20dir%2Fx.png&token=a%20b',
    );
    expect(c.eventsUrl()).toBe('ws://h:1/events?token=a%20b');
  });
});

class FakeSocket {
  static all: FakeSocket[] = [];
  onopen?: () => void;
  onclose?: (e: any) => void;
  onmessage?: (m: any) => void;
  onerror?: () => void;
  constructor(public url: string) {
    FakeSocket.all.push(this);
  }
  close() {
    this.onclose?.({ code: 1000 });
  }
}

describe('events', () => {
  it('delivers events and reconnects with backoff', async () => {
    vi.useFakeTimers();
    FakeSocket.all = [];
    const events: any[] = [];
    const states: string[] = [];
    const c = new RemoteClient({ base: 'http://h:1', token: 't', WebSocket: FakeSocket });
    const sub = c.subscribe(
      (e) => events.push(e),
      (s) => states.push(s),
    );
    FakeSocket.all[0]!.onopen!();
    FakeSocket.all[0]!.onmessage!({ data: '{"type":"changed","threads":["a"]}' });
    FakeSocket.all[0]!.onmessage!({ data: 'garbage' });
    expect(events).toEqual([{ type: 'changed', threads: ['a'] }]);
    FakeSocket.all[0]!.onclose!({ code: 1006 });
    expect(FakeSocket.all).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(1000);
    expect(FakeSocket.all).toHaveLength(2);
    sub.close();
    FakeSocket.all[1]!.onclose!({ code: 1006 });
    await vi.advanceTimersByTimeAsync(20000);
    expect(FakeSocket.all).toHaveLength(2);
    expect(states).toContain('open');
    vi.useRealTimers();
  });
});

describe('machine scoping of urls', () => {
  it('two clients never share a base or a token in media and file urls', () => {
    const a = new RemoteClient({ base: 'http://a:1', token: 'TA' });
    const b = new RemoteClient({ base: 'http://b:2', token: 'TB' });
    for (const url of [a.mediaUrl('t', 's'), a.fileUrl('t', 'p'), a.eventsUrl()]) {
      expect(url).toContain('TA');
      expect(url).not.toContain('TB');
    }
    expect(b.mediaUrl('t', 's')).toBe('http://b:2/media/t/s?token=TB');
    expect(b.eventsUrl()).toBe('ws://b:2/events?token=TB');
  });
  it('a pairing link carries the machine id, and an older link without one still parses', () => {
    expect(
      parsePairingLink('orochi://pair?host=1.2.3.4&port=5&code=c&name=m&id=abcdef0123456789')?.id,
    ).toBe('abcdef0123456789');
    expect(parsePairingLink('orochi://pair?host=1.2.3.4&port=5&code=c')?.id).toBeUndefined();
    expect(
      parsePairingLink('orochi://pair?host=1.2.3.4&port=5&code=c&id=bad id!')?.id,
    ).toBeUndefined();
  });
});
