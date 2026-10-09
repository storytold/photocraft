#!/usr/bin/env python3
"""Serve a directory on 127.0.0.1 with the headers the threads build needs (COOP/COEP, wasm MIME).

    python3 packaging/web-threads/serve.py dist/web-threads [port]
"""
import functools
import http.server
import sys


class Handler(http.server.SimpleHTTPRequestHandler):
    extensions_map = {**http.server.SimpleHTTPRequestHandler.extensions_map, ".wasm": "application/wasm", ".js": "text/javascript"}

    def end_headers(self):
        self.send_header("Cross-Origin-Opener-Policy", "same-origin")
        self.send_header("Cross-Origin-Embedder-Policy", "require-corp")
        self.send_header("Cross-Origin-Resource-Policy", "same-origin")
        self.send_header("Cache-Control", "no-cache")
        super().end_headers()


def main():
    root = sys.argv[1] if len(sys.argv) > 1 else "dist/web-threads"
    port = int(sys.argv[2]) if len(sys.argv) > 2 else 8777
    handler = functools.partial(Handler, directory=root)
    with http.server.ThreadingHTTPServer(("127.0.0.1", port), handler) as httpd:
        print(f"serving {root} on http://127.0.0.1:{port}/")
        httpd.serve_forever()


if __name__ == "__main__":
    main()
