"""A System One server for tests: what `llama-server` and OpenRouter answer on /v1/systemone.

Started with no arguments it listens on a free loopback port and prints its base URL on the
first line of stdout. Started as `llama-server` is (`-m <gguf> --host 127.0.0.1 --port N`) it
listens on that port, which is how the local backend's supervisor runs it.

  POST /v1/decisions  the same, in OpenAI's Decisions shape: `questions` a list of
                      predicate / choice (`choices`) / score (`levels`), answers a list.
  POST /v1/systemone  answers every question asked: a choice picks the option named in
                      MOCK_SYSTEMONE_ANSWERS (a JSON file, {question: answer}) or the first,
                      a score answers what is named there or 1.0, a noul 0.1. Every request
                      is appended to MOCK_SYSTEMONE_LOG as one JSON line with its
                      Authorization header. MOCK_SYSTEMONE_STATUS answers that status instead.
  GET  /health        {"status": "ok"}
  GET  /files/<name>  a file from MOCK_SYSTEMONE_FILES, honoring a Range header, so an
                      interrupted download can resume; MOCK_SYSTEMONE_CUT=<bytes> cuts the
                      first full response short after that many bytes.
"""

import json
import os
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

CUT = {"left": int(os.environ.get("MOCK_SYSTEMONE_CUT", "0") or 0)}


QUEUES = {}


def answers():
    path = os.environ.get("MOCK_SYSTEMONE_ANSWERS")
    if path and os.path.exists(path):
        with open(path) as f:
            return json.load(f)
    return {}


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def send(self, status, body, kind="application/json"):
        data = body if isinstance(body, bytes) else json.dumps(body).encode()
        self.send_response(status)
        self.send_header("Content-Type", kind)
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        if self.path == "/health":
            return self.send(200, {"status": "ok"})
        if self.path.startswith("/files/"):
            root = os.environ.get("MOCK_SYSTEMONE_FILES", "")
            path = os.path.join(root, os.path.basename(self.path))
            if not os.path.isfile(path):
                return self.send(404, {"error": "missing"})
            with open(path, "rb") as f:
                data = f.read()
            start = 0
            header = self.headers.get("Range")
            if header and header.startswith("bytes="):
                start = int(header[6:].split("-")[0])
                self.send_response(206)
                self.send_header(
                    "Content-Range", f"bytes {start}-{len(data) - 1}/{len(data)}"
                )
            else:
                self.send_response(200)
            body = data[start:]
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            if CUT["left"] and not header:
                left, CUT["left"] = CUT["left"], 0
                self.wfile.write(body[:left])
                self.wfile.flush()
                self.close_connection = True
                return
            self.wfile.write(body)
            return
        self.send(404, {"error": "not found"})

    def do_POST(self):
        length = int(self.headers.get("Content-Length", "0"))
        request = json.loads(self.rfile.read(length) or b"{}")
        log = os.environ.get("MOCK_SYSTEMONE_LOG")
        if log:
            with open(log, "a") as f:
                f.write(
                    json.dumps(
                        {
                            "path": self.path,
                            "authorization": self.headers.get("Authorization"),
                            "request": request,
                        }
                    )
                    + "\n"
                )
        status = int(os.environ.get("MOCK_SYSTEMONE_STATUS", "200"))
        if status != 200:
            return self.send(status, {"error": "refused"})
        wanted = answers()
        # An answer that is a list is given one per request for that question, in order: the
        # last one again once they run out.
        for name, value in list(wanted.items()):
            if isinstance(value, list):
                queue = QUEUES.setdefault(name, list(value))
                wanted[name] = queue.pop(0) if len(queue) > 1 else queue[0]
        if self.path == "/v1/decisions":
            out = []
            for question in request.get("questions") or []:
                name, kind = question.get("name"), question.get("type")
                if kind == "predicate":
                    out.append({"type": "predicate", "name": name, "probability": float(wanted.get(name, 0.1))})
                elif kind == "choice":
                    values = [c["value"] for c in question.get("choices") or []]
                    pick = wanted.get(name, values[0] if values else None)
                    p = float(wanted.get(name + ".p", 0.9))
                    out.append({"type": "choice", "name": name, "choice": pick, "confidence": p,
                                "probabilities": [{"value": v, "probability": p if v == pick else 0.0} for v in values]})
                elif kind == "score":
                    out.append({"type": "score", "name": name, "score": float(wanted.get(name, 1.0)), "confidence": 0.5})
            return self.send(200, {"answers": out})
        if self.path != "/v1/systemone":
            return self.send(404, {"error": "not found"})
        out = {}
        for name, question in (request.get("questions") or {}).items():
            kind = question.get("type")
            if kind == "choice":
                options = list((question.get("criteria") or {}).keys())
                pick = wanted.get(name, options[0] if options else None)
                p = float(wanted.get(name + ".p", 0.9))
                probs = {o: (p if o == pick else (1 - p) / max(1, len(options) - 1)) for o in options}
                out[name] = {"type": "choice", "choice": pick, "probabilities": probs, "confidence": p}
            elif kind == "score":
                out[name] = {"type": "score", "score": float(wanted.get(name, 1.0))}
            elif kind == "noul":
                out[name] = {"type": "noul", "noul": float(wanted.get(name, 0.1))}
        self.send(200, {"model": request.get("model"), "answers": out})


def main():
    port = 0
    args = sys.argv[1:]
    if "--port" in args:
        port = int(args[args.index("--port") + 1])
    server = ThreadingHTTPServer(("127.0.0.1", port), Handler)
    print(f"http://127.0.0.1:{server.server_address[1]}", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
