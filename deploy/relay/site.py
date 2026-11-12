#!/usr/bin/env python3
import argparse
import re
import shutil
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
    '/applebridge/selfhost/install.sh': ('applebridge/selfhost/install.sh', 'text/plain; charset=utf-8'),
}
PROJECT = 'https://gitlab.com/johansardar/apple-bridge'
IPHONE = 'https://gitlab.com/johansardar/apple-bridge-iphone'
redirects = {
    '/applebridge/source': PROJECT,
    '/applebridge/issues': PROJECT + '/-/issues',
    '/applebridge/issues/new': PROJECT + '/-/issues/new',
    '/applebridge/pipelines': PROJECT + '/-/pipelines',
    '/applebridge/pipeline.svg': PROJECT + '/badges/main/pipeline.svg',
    '/applebridge/iphone': IPHONE,
}
redirect_prefixes = {
    '/applebridge/source/': PROJECT + '/-/blob/main/',
    '/applebridge/iphone/': IPHONE + '/-/blob/main/',
}
REPO_PATH = re.compile(r'[A-Za-z0-9_][A-Za-z0-9_./-]*')
DOWNLOAD = re.compile(r'/applebridge/download/((?:[A-Za-z0-9][A-Za-z0-9._-]*/)?[A-Za-z0-9][A-Za-z0-9._-]*)')
DOWNLOAD_TYPES = {'.tgz': 'application/gzip', '.gz': 'application/gzip', '.sha256': 'text/plain; charset=utf-8'}


def redirect_target(path):
    if path in redirects:
        return redirects[path]
    for prefix, target in redirect_prefixes.items():
        rest = path[len(prefix):]
        if path.startswith(prefix) and REPO_PATH.fullmatch(rest) and '..' not in rest.split('/'):
            return target + rest
    return None


def download_file(path):
    match = DOWNLOAD.fullmatch(path)
    if match is None or '..' in match.group(1):
        return None
    name = match.group(1)
    content_type = DOWNLOAD_TYPES.get(Path(name).suffix, 'text/plain; charset=utf-8' if '/' not in name else None)
    file = args.root / 'applebridge/download' / name
    if content_type is None or not file.is_file():
        return None
    return file, content_type


class Site(BaseHTTPRequestHandler):
    def do_HEAD(self):
        self.serve(False)

    def do_GET(self):
        self.serve(True)

    def serve(self, body):
        path = self.path.split('?', 1)[0]
        if self.headers.get('Host') != 'eilmer.si':
            self.send_error(404)
            return
        target = redirect_target(path)
        if target is not None:
            self.send_response(302)
            self.send_header('Location', target)
            self.send_header('Content-Length', '0')
            self.end_headers()
            return
        route = routes.get(path)
        found = (args.root / route[0], route[1]) if route else download_file(path)
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
