#!/usr/bin/env python3
"""MiniBrowser dev server with CORS proxy for fetching external URLs."""
import http.server
import socketserver
import urllib.request
import urllib.parse
import json

PORT = 9090

class Handler(http.server.SimpleHTTPRequestHandler):
    extensions_map = {
        **http.server.SimpleHTTPRequestHandler.extensions_map,
        '.wasm': 'application/wasm',
        '.js': 'application/javascript',
    }

    def end_headers(self):
        self.send_header('Access-Control-Allow-Origin', '*')
        self.send_header('Access-Control-Allow-Methods', 'GET, POST, OPTIONS')
        self.send_header('Access-Control-Allow-Headers', 'Content-Type')
        super().end_headers()

    def do_OPTIONS(self):
        self.send_response(200)
        self.end_headers()

    def do_GET(self):
        # Proxy endpoint: /proxy?url=https://example.com
        if self.path.startswith('/proxy?url='):
            url = self.path[len('/proxy?url='):]
            url = urllib.parse.unquote(url)
            try:
                req = urllib.request.Request(url, headers={'User-Agent': 'Mozilla/5.0'})
                with urllib.request.urlopen(req, timeout=10) as resp:
                    data = resp.read()
                    content_type = resp.headers.get('Content-Type', 'text/html')
                self.send_response(200)
                self.send_header('Content-Type', content_type)
                self.end_headers()
                self.wfile.write(data)
            except Exception as e:
                self.send_response(502)
                self.send_header('Content-Type', 'text/plain')
                self.end_headers()
                self.wfile.write(f'Proxy error: {e}'.encode())
            return
        # Default: serve static files
        super().do_GET()

with socketserver.TCPServer(("0.0.0.0", PORT), Handler) as httpd:
    print(f"minibrowser dev server: http://0.0.0.0:{PORT}")
    print(f"CORS proxy: http://0.0.0.0:{PORT}/proxy?url=<URL>")
    httpd.serve_forever()
