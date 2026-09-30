#!/usr/bin/python3
"""The sandboxes' HTTP gateway, run inside the host VM by `orochi sandbox up`.

`http://<port>-<project>.localhost:<gateway>/` on the Mac reaches `<project>.sbx:<port>`: any
browser and curl resolve `*.localhost` to the loopback address themselves, Lima forwards the
gateway's loopback port to the Mac's, and Incus's DNS knows each sandbox by name. So every
project's services are reachable at once, and nothing on the Mac is configured or needs root.

It reads a request's head only to find where it goes, then passes bytes through unchanged in
both directions, so a WebSocket upgrade works as any other request does. HTTP only: a database
port is reached with `orochi sandbox focus`.
"""
import asyncio
import os
import re

PORT = int(os.environ.get("SBX_GATEWAY_PORT", "1355"))
LISTEN = os.environ.get("SBX_GATEWAY_LISTEN", "127.0.0.1")
DOMAIN = os.environ.get("SBX_GATEWAY_DOMAIN", "sbx")
# Tests connect every project to one address instead of `<project>.<domain>`.
UPSTREAM = os.environ.get("SBX_GATEWAY_UPSTREAM")
HOST = re.compile(r"^(\d{1,5})-([a-z][a-z0-9-]{0,62})\.localhost(?::\d{1,5})?$")
HEAD_LIMIT = 64 * 1024


def route(host):
    """(project, port) for a Host header naming one, else None."""
    match = HOST.match(host.strip().lower())
    if not match:
        return None
    port = int(match.group(1))
    if not 1 <= port <= 65535 or match.group(2).endswith("-"):
        return None
    return match.group(2), port


def header(head, name):
    for line in head.split(b"\r\n")[1:]:
        key, _, value = line.partition(b":")
        if key.strip().lower() == name:
            return value.strip().decode("latin-1")
    return None


async def refuse(writer, status, reason, text):
    body = (text + "\n").encode()
    writer.write(
        f"HTTP/1.1 {status} {reason}\r\nContent-Type: text/plain; charset=utf-8\r\n"
        f"Content-Length: {len(body)}\r\nConnection: close\r\n\r\n".encode() + body
    )
    try:
        await writer.drain()
    finally:
        writer.close()


async def pipe(reader, writer):
    try:
        while data := await reader.read(65536):
            writer.write(data)
            await writer.drain()
    except (ConnectionError, OSError):
        pass
    finally:
        try:
            writer.close()
        except (ConnectionError, OSError):
            pass


async def handle(reader, writer):
    try:
        head = await asyncio.wait_for(reader.readuntil(b"\r\n\r\n"), 30)
    except (asyncio.IncompleteReadError, asyncio.LimitOverrunError, asyncio.TimeoutError, ConnectionError):
        writer.close()
        return
    target = route(header(head, b"host") or "")
    if target is None:
        await refuse(
            writer, 400, "Bad Request",
            f"Open a sandbox's service as http://<port>-<project>.localhost:{PORT}/",
        )
        return
    project, port = target
    try:
        upstream_reader, upstream_writer = await asyncio.wait_for(
            asyncio.open_connection(UPSTREAM or f"{project}.{DOMAIN}", port), 10
        )
    except (OSError, asyncio.TimeoutError) as error:
        await refuse(
            writer, 502, "Bad Gateway",
            f"Nothing answered at {project}:{port} ({type(error).__name__}). Is the sandbox "
            f"running and the service listening on 0.0.0.0? A service bound to 127.0.0.1 "
            f"inside is reached with `orochi sandbox focus`.",
        )
        return
    upstream_writer.write(head)
    await upstream_writer.drain()
    await asyncio.gather(pipe(reader, upstream_writer), pipe(upstream_reader, writer))


async def main():
    server = await asyncio.start_server(handle, LISTEN, PORT, limit=HEAD_LIMIT)
    async with server:
        await server.serve_forever()


if __name__ == "__main__":
    asyncio.run(main())
