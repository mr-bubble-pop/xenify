#!/usr/bin/env python3
"""Serves the browser build of Xenify Write (target/write-web) on this
computer, the way a real host should: pre-compressed .br/.gz copies when the
browser accepts them, the right types, and the cache headers from _headers.

    scripts/serve-write-web.py [--port 8080] [--dir target/write-web]

Then open http://localhost:8080/ (service workers need https or localhost).
With ?debug in the address, page errors are printed here.
"""
import argparse
import http.server
import mimetypes
import os
import sys
import urllib.parse

TYPES = {".wasm": "application/wasm", ".js": "text/javascript", ".mjs": "text/javascript",
         ".webmanifest": "application/manifest+json", ".bin": "application/octet-stream",
         ".md": "text/markdown; charset=utf-8", ".json": "application/json"}
FOREVER = ("pkg/", "fonts.bin", "spotter/model.part")


class Handler(http.server.SimpleHTTPRequestHandler):
    def log_message(self, fmt, *args):
        if "--verbose" in sys.argv:
            super().log_message(fmt, *args)

    def do_GET(self):
        url = urllib.parse.urlsplit(self.path)
        if url.path == "/__error":
            print("page error:", urllib.parse.unquote(url.query), flush=True)
            self.send_response(204)
            self.end_headers()
            return
        rel = urllib.parse.unquote(url.path).lstrip("/") or "index.html"
        path = os.path.normpath(os.path.join(self.directory, rel))
        if not path.startswith(os.path.abspath(self.directory)) or not os.path.isfile(path):
            self.send_error(404)
            return
        accepts = self.headers.get("Accept-Encoding", "")
        body, encoding = path, None
        for enc, ext in (("br", ".br"), ("gzip", ".gz")):
            if enc in accepts and os.path.isfile(path + ext):
                body, encoding = path + ext, enc
                break
        ext = os.path.splitext(path)[1]
        kind = TYPES.get(ext) or mimetypes.guess_type(path)[0] or "application/octet-stream"
        if kind.startswith("text/") and "charset" not in kind:
            kind += "; charset=utf-8"
        forever = rel.startswith(FOREVER) and rel.endswith((".wasm", ".bin")) or "model.part" in rel
        self.send_response(200)
        self.send_header("Content-Type", kind)
        self.send_header("Content-Length", str(os.path.getsize(body)))
        self.send_header("Cache-Control", "public, max-age=31536000, immutable" if forever else "no-cache")
        self.send_header("X-Content-Type-Options", "nosniff")
        if encoding:
            self.send_header("Content-Encoding", encoding)
            self.send_header("Vary", "Accept-Encoding")
        self.end_headers()
        with open(body, "rb") as f:
            self.wfile.write(f.read())


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--port", type=int, default=8080)
    ap.add_argument("--dir", default=os.path.join(here, "..", "target", "write-web"))
    ap.add_argument("--verbose", action="store_true")
    a = ap.parse_args()
    root = os.path.abspath(a.dir)
    handler = lambda *x, **k: Handler(*x, directory=root, **k)
    server = http.server.ThreadingHTTPServer(("127.0.0.1", a.port), handler)
    print(f"Xenify Write: http://localhost:{a.port}/  (serving {root})", flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
