"""Deterministic MCP server behind OAuth, with its own authorization server.

Prints its base URL on stdout, then serves:
  POST /mcp                                  401 unless the bearer token is the current one
  GET  /.well-known/oauth-protected-resource  RFC 9728 metadata naming this authorization server
  GET  /.well-known/oauth-authorization-server RFC 8414 metadata
  POST /register                              RFC 7591 dynamic registration
  GET  /authorize                             redirects straight back to the loopback redirect
  POST /token                                 authorization_code and refresh_token grants

MOCK_OAUTH_NO_REGISTER=1 drops the registration endpoint, so a client with no way in says so.
MOCK_OAUTH_NO_PKCE=1 drops code_challenge_methods_supported, which a client must refuse.
MOCK_OAUTH_SCOPE puts a scope in the 401 challenge, which outranks scopes_supported.
MOCK_OAUTH_ISSUER_PATH gives the issuer a path and serves its metadata only where OpenID Connect
appends to that path — the last place the MCP specification has a client look.
"""
import hashlib
import base64
import json
import os
import sys
import threading
import urllib.parse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

state = {"access": "first-token", "issued": 0, "codes": {}, "clients": 0, "challenges": {}}
lock = threading.Lock()


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def base(self):
        return f"http://127.0.0.1:{self.server.server_address[1]}"

    def reply(self, code, body, headers=()):
        payload = json.dumps(body).encode() if not isinstance(body, bytes) else body
        self.send_response(code)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(payload)))
        for name, value in headers:
            self.send_header(name, value)
        self.end_headers()
        self.wfile.write(payload)

    def do_GET(self):
        url = urllib.parse.urlparse(self.path)
        query = dict(urllib.parse.parse_qsl(url.query))
        if url.path == "/mcp":
            # No server-initiated stream: streamable HTTP says so with 405.
            return self.reply(405, {"error": "method_not_allowed"})
        issuer_path = os.environ.get("MOCK_OAUTH_ISSUER_PATH", "")
        if url.path == "/.well-known/oauth-protected-resource":
            return self.reply(200, {"resource": f"{self.base()}/mcp",
                                    "authorization_servers": [self.base() + issuer_path],
                                    "scopes_supported": ["mcp.read"]})
        metadata_at = (f"{issuer_path}/.well-known/openid-configuration" if issuer_path
                       else "/.well-known/oauth-authorization-server")
        if url.path == metadata_at:
            body = {"issuer": self.base() + issuer_path,
                    "authorization_endpoint": f"{self.base()}/authorize",
                    "token_endpoint": f"{self.base()}/token",
                    "code_challenge_methods_supported": ["S256"]}
            if os.environ.get("MOCK_OAUTH_NO_PKCE"):
                del body["code_challenge_methods_supported"]
            if not os.environ.get("MOCK_OAUTH_NO_REGISTER"):
                body["registration_endpoint"] = f"{self.base()}/register"
            return self.reply(200, body)
        if url.path == "/authorize":
            # Everything the client must send is checked here, not assumed.
            for required in ("client_id", "redirect_uri", "state", "code_challenge", "resource"):
                if required not in query:
                    return self.reply(400, {"error": "invalid_request", "missing": required})
            if query.get("code_challenge_method") != "S256":
                return self.reply(400, {"error": "invalid_request"})
            with lock:
                code = f"code-{len(state['codes'])}"
                state["codes"][code] = query["code_challenge"]
            target = query["redirect_uri"] + "?" + urllib.parse.urlencode(
                {"code": code, "state": query["state"]})
            self.send_response(302)
            self.send_header("location", target)
            self.send_header("content-length", "0")
            self.end_headers()
            return
        self.reply(404, {"error": "not_found"})

    def do_POST(self):
        url = urllib.parse.urlparse(self.path)
        length = int(self.headers.get("content-length") or 0)
        body = self.rfile.read(length).decode()
        if url.path == "/mcp":
            token = (self.headers.get("authorization") or "").removeprefix("Bearer ")
            with lock:
                current = state["access"]
            if token != current:
                scope = os.environ.get("MOCK_OAUTH_SCOPE")
                challenge = f'Bearer resource_metadata="{self.base()}/.well-known/oauth-protected-resource"'
                if scope:
                    challenge += f', scope="{scope}"'
                return self.reply(401, {"error": "unauthorized"}, [("www-authenticate", challenge)])
            # Enough of a streamable-HTTP MCP server for a real agent to list and call one tool.
            request = json.loads(body or "{}")
            method, request_id = request.get("method"), request.get("id")
            if log_path := os.environ.get("MOCK_OAUTH_LOG"):
                with open(log_path, "a") as log:
                    log.write(json.dumps({"method": method, "token": token}) + "\n")
            if request_id is None:
                self.send_response(202)
                self.send_header("content-length", "0")
                self.end_headers()
                return
            result = {"protocolVersion": "2025-06-18"}
            if method == "initialize":
                result = {"protocolVersion": request.get("params", {}).get("protocolVersion", "2025-06-18"),
                          "capabilities": {"tools": {}}, "serverInfo": {"name": "mock-oauth", "version": "1"}}
            elif method == "tools/list":
                result = {"tools": [{"name": "orochi_probe", "description": "Returns the verification marker.",
                                     "inputSchema": {"type": "object", "properties": {}}}]}
            elif method == "tools/call":
                result = {"content": [{"type": "text", "text": os.environ.get("MOCK_OAUTH_MARKER", "PROBE-HTTP")}],
                          "isError": False}
            elif method not in (None, "ping"):
                return self.reply(200, {"jsonrpc": "2.0", "id": request_id,
                                        "error": {"code": -32601, "message": "method not found"}})
            return self.reply(200, {"jsonrpc": "2.0", "id": request_id, "result": result})
        if url.path == "/register":
            with lock:
                state["clients"] += 1
                client = f"client-{state['clients']}"
            return self.reply(201, {"client_id": client, "redirect_uris": json.loads(body)["redirect_uris"]})
        if url.path == "/token":
            form = dict(urllib.parse.parse_qsl(body))
            with lock:
                if form.get("grant_type") == "authorization_code":
                    challenge = state["codes"].pop(form.get("code", ""), None)
                    if challenge is None:
                        return self.reply(400, {"error": "invalid_grant"})
                    digest = hashlib.sha256(form.get("code_verifier", "").encode()).digest()
                    if base64.urlsafe_b64encode(digest).decode().rstrip("=") != challenge:
                        return self.reply(400, {"error": "invalid_grant", "detail": "pkce"})
                elif form.get("grant_type") == "refresh_token":
                    if form.get("refresh_token") != "refresh-me":
                        return self.reply(400, {"error": "invalid_grant"})
                else:
                    return self.reply(400, {"error": "unsupported_grant_type"})
                if form.get("resource", "") != f"{self.base()}/mcp":
                    return self.reply(400, {"error": "invalid_target"})
                state["issued"] += 1
                state["access"] = f"token-{state['issued']}"
                issued = state["access"]
            return self.reply(200, {"access_token": issued, "token_type": "Bearer",
                                    "expires_in": 3600, "refresh_token": "refresh-me"})
        self.reply(404, {"error": "not_found"})


server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
with lock:
    state["access"] = "unissued"
print(f"http://127.0.0.1:{server.server_address[1]}", flush=True)
server.serve_forever()
