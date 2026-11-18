// Web-only test aid: puts CORS headers in front of a real orochi-remote so the Expo web build
// (a browser) can talk to it. The server itself sends none, on purpose. Not used on devices.
import http from 'node:http';
import net from 'node:net';

const [, , listen = '47822', target = '47821'] = process.argv;
const cors = {
  'access-control-allow-origin': '*',
  'access-control-allow-headers': 'authorization, content-type',
  'access-control-allow-methods': 'GET, POST, OPTIONS',
  'access-control-expose-headers': 'content-range, accept-ranges, content-length',
};
const server = http.createServer((req, res) => {
  if (req.method === 'OPTIONS') {
    res.writeHead(204, cors);
    return res.end();
  }
  const up = http.request(
    {
      host: '127.0.0.1',
      port: Number(target),
      path: req.url,
      method: req.method,
      headers: req.headers,
    },
    (r) => {
      res.writeHead(r.statusCode ?? 502, { ...r.headers, ...cors });
      r.pipe(res);
    },
  );
  up.on('error', () => res.writeHead(502, cors).end());
  req.pipe(up);
});
server.on('upgrade', (req, socket, head) => {
  const up = net.connect(Number(target), '127.0.0.1', () => {
    up.write(
      `${req.method} ${req.url} HTTP/1.1\r\n${Object.entries(req.headers)
        .map(([k, v]) => `${k}: ${v}`)
        .join('\r\n')}\r\n\r\n`,
    );
    up.write(head);
    socket.pipe(up).pipe(socket);
  });
  up.on('error', () => socket.destroy());
});
server.listen(Number(listen), '127.0.0.1', () => console.log(`cors proxy ${listen} -> ${target}`));
