#!/usr/bin/env python3
import argparse
import re
import shutil
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

parser = argparse.ArgumentParser()
parser.add_argument('root', type=Path)
parser.add_argument('--port', type=int, default=8793)
parser.add_argument('--downloads', type=Path, required=True)
args = parser.parse_args()
routes = {
    '/': ('index.html', 'text/html; charset=utf-8'),
    '/applebridge': ('applebridge/index.html', 'text/html; charset=utf-8'),
    '/applebridge/': ('applebridge/index.html', 'text/html; charset=utf-8'),
    '/assets/site.css': ('assets/site.css', 'text/css; charset=utf-8'),
    '/applebridge/setup.md': ('applebridge/setup.md', 'text/plain; charset=utf-8'),
    '/applebridge/install': ('applebridge/install', 'text/plain; charset=utf-8'),
}
DOWNLOAD = re.compile(r'/applebridge/download/((?:[A-Za-z0-9][A-Za-z0-9._-]*/)?[A-Za-z0-9][A-Za-z0-9._-]*)')
DOWNLOAD_TYPES = {'.tgz': 'application/gzip', '.gz': 'application/gzip', '.sha256': 'text/plain; charset=utf-8'}


def download_file(path):
    match = DOWNLOAD.fullmatch(path)
    if match is None or '..' in match.group(1):
        return None
    name = match.group(1)
    content_type = DOWNLOAD_TYPES.get(Path(name).suffix, 'text/plain; charset=utf-8' if '/' not in name else None)
    base = args.downloads.resolve()
    file = (base / name).resolve()
    if content_type is None or not file.is_relative_to(base) or not file.is_file():
        return None
    return file, content_type


def document_file(path):
    if not path.startswith('/applebridge/docs/') or not path.endswith('.md'):
        return None
    base = (args.root / 'applebridge/docs').resolve()
    file = (args.root / path.lstrip('/')).resolve()
    if not file.is_relative_to(base) or not file.is_file():
        return None
    return file, 'text/plain; charset=utf-8'


class Site(BaseHTTPRequestHandler):
    timeout = 30

    def do_HEAD(self):
        self.serve(False)

    def do_GET(self):
        self.serve(True)

    def serve(self, body):
        path = self.path.split('?', 1)[0]
        if self.headers.get('Host') != 'eilmer.si':
            self.send_error(404)
            return
        route = routes.get(path)
        found = (args.root / route[0], route[1]) if route else document_file(path) or download_file(path)
        if found is None:
            self.send_error(404)
            return
        file, content_type = found
        self.send_response(200)
        self.send_header('Content-Type', content_type)
        self.send_header('Content-Length', str(file.stat().st_size))
        self.send_header('X-Content-Type-Options', 'nosniff')
        self.send_header('Content-Security-Policy', "default-src 'self'; frame-ancestors 'none'")
        self.end_headers()
        if body:
            with file.open('rb') as source:
                shutil.copyfileobj(source, self.wfile)


ThreadingHTTPServer(('127.0.0.1', args.port), Site).serve_forever()
