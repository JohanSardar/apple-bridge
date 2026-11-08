#!/usr/bin/env python3
import argparse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

parser = argparse.ArgumentParser()
parser.add_argument('root', type=Path)
parser.add_argument('--port', type=int, default=8793)
args = parser.parse_args()
routes = {
    '/': ('index.html', 'text/html; charset=utf-8'),
    '/applebridge': ('applebridge/index.html', 'text/html; charset=utf-8'),
    '/applebridge/': ('applebridge/index.html', 'text/html; charset=utf-8'),
    '/assets/site.css': ('assets/site.css', 'text/css; charset=utf-8'),
    '/applebridge/install': ('applebridge/install', 'text/plain; charset=utf-8'),
}


class Site(BaseHTTPRequestHandler):
    def do_HEAD(self):
        self.serve(False)

    def do_GET(self):
        self.serve(True)

    def serve(self, body):
        route = routes.get(self.path.split('?', 1)[0])
        if self.headers.get('Host') != 'eilmer.si' or route is None:
            self.send_error(404)
            return
        data = (args.root / route[0]).read_bytes()
        self.send_response(200)
        self.send_header('Content-Type', route[1])
        self.send_header('Content-Length', str(len(data)))
        self.send_header('X-Content-Type-Options', 'nosniff')
        self.send_header('Content-Security-Policy', "default-src 'self'; frame-ancestors 'none'")
        self.end_headers()
        if body:
            self.wfile.write(data)


ThreadingHTTPServer(('127.0.0.1', args.port), Site).serve_forever()
