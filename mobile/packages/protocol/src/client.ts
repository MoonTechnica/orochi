import type { Health, Me, PairResult, ServerEvent } from './types';
import { baseUrl } from './pairing';

export class RemoteError extends Error {
  constructor(
    message: string,
    readonly status: number,
  ) {
    super(message);
  }
  get unauthorized() {
    return this.status === 401;
  }
}

export interface ClientOptions {
  base: string;
  token?: string | null;
  fetch?: typeof fetch;
  WebSocket?: any;
  timeoutMs?: number;
}

export type ConnectionState = 'connecting' | 'open' | 'closed';

export interface EventSubscription {
  close(): void;
}

export class RemoteClient {
  readonly base: string;
  token: string | null;
  private fetchImpl: typeof fetch;
  private WS: any;
  private timeoutMs: number;

  constructor(options: ClientOptions) {
    this.base = options.base.replace(/\/+$/, '');
    this.token = options.token ?? null;
    this.fetchImpl = options.fetch ?? ((...args) => fetch(...args));
    this.WS = options.WebSocket ?? (globalThis as any).WebSocket;
    this.timeoutMs = options.timeoutMs ?? 15000;
  }

  static for(
    host: string,
    port: number,
    token?: string | null,
    extra: Partial<ClientOptions> = {},
  ) {
    return new RemoteClient({ base: baseUrl(host, port), token, ...extra });
  }

  private async request<T>(path: string, init: RequestInit = {}, auth = true): Promise<T> {
    const headers: Record<string, string> = { Accept: 'application/json' };
    if (init.body) headers['Content-Type'] = 'application/json';
    if (auth && this.token) headers.Authorization = `Bearer ${this.token}`;
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), this.timeoutMs);
    let response: Response;
    try {
      response = await this.fetchImpl(`${this.base}${path}`, {
        ...init,
        headers: { ...headers, ...(init.headers as Record<string, string> | undefined) },
        signal: controller.signal,
      });
    } catch (error) {
      throw new RemoteError(
        (error as Error)?.name === 'AbortError'
          ? 'timed out'
          : `unreachable: ${(error as Error)?.message ?? error}`,
        0,
      );
    } finally {
      clearTimeout(timer);
    }
    const raw = await response.text();
    let body: any = null;
    if (raw) {
      try {
        body = JSON.parse(raw);
      } catch {
        body = raw;
      }
    }
    if (!response.ok) {
      const message =
        typeof body === 'object' && body?.error ? body.error : String(body ?? response.statusText);
      throw new RemoteError(message, response.status);
    }
    return body as T;
  }

  health() {
    return this.request<Health>('/health', {}, false);
  }

  /** `deviceId` is this install's stable id: pairing again with it replaces the old device row. */
  async pair(code: string, device: string, deviceId?: string): Promise<PairResult> {
    return this.request<PairResult>(
      '/pair',
      {
        method: 'POST',
        body: JSON.stringify(deviceId ? { code, device, device_id: deviceId } : { code, device }),
      },
      false,
    );
  }

  me() {
    return this.request<Me>('/me');
  }

  /** `/rpc/<command>`: a whitelisted view command, arguments as the window passes them. */
  rpc<T = unknown>(command: string, args: Record<string, unknown> = {}): Promise<T> {
    return this.request<T>(`/rpc/${encodeURIComponent(command)}`, {
      method: 'POST',
      body: JSON.stringify(args),
    });
  }

  private withToken(url: string) {
    return this.token
      ? `${url}${url.includes('?') ? '&' : '?'}token=${encodeURIComponent(this.token)}`
      : url;
  }

  mediaUrl(thread: string, sha: string) {
    return this.withToken(
      `${this.base}/media/${encodeURIComponent(thread)}/${encodeURIComponent(sha)}`,
    );
  }

  fileUrl(thread: string, path: string) {
    return this.withToken(
      `${this.base}/file?thread=${encodeURIComponent(thread)}&path=${encodeURIComponent(path)}`,
    );
  }

  eventsUrl() {
    return this.withToken(`${this.base.replace(/^http/, 'ws')}/events`);
  }

  /** WebSocket with automatic reconnect (exponential backoff, capped). */
  subscribe(
    onEvent: (event: ServerEvent) => void,
    onState?: (state: ConnectionState) => void,
    onUnauthorized?: () => void,
  ): EventSubscription {
    let closed = false;
    let socket: any = null;
    let attempt = 0;
    let timer: ReturnType<typeof setTimeout> | null = null;
    const open = () => {
      if (closed) return;
      onState?.('connecting');
      try {
        socket = new this.WS(this.eventsUrl());
      } catch {
        schedule();
        return;
      }
      socket.onopen = () => {
        attempt = 0;
        onState?.('open');
      };
      socket.onmessage = (message: { data: unknown }) => {
        try {
          const event = JSON.parse(String(message.data)) as ServerEvent;
          if (event && typeof event.type === 'string') onEvent(event);
        } catch {
          /* a frame that is not ours */
        }
      };
      socket.onclose = (event: { code?: number }) => {
        onState?.('closed');
        if (event?.code === 4401 || event?.code === 1008) onUnauthorized?.();
        schedule();
      };
      socket.onerror = () => {
        try {
          socket?.close();
        } catch {
          /* already closed */
        }
      };
    };
    const schedule = () => {
      if (closed || timer) return;
      const wait = Math.min(15000, 500 * 2 ** Math.min(attempt++, 5)) + Math.random() * 250;
      timer = setTimeout(() => {
        timer = null;
        open();
      }, wait);
    };
    open();
    return {
      close() {
        closed = true;
        if (timer) clearTimeout(timer);
        try {
          socket?.close();
        } catch {
          /* ignore */
        }
      },
    };
  }
}
