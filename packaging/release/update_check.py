#!/usr/bin/env python3
import argparse
import base64
from functools import partial
import http.server
import os
import signal
from pathlib import Path
import plistlib
import shutil
import subprocess
import tempfile
import threading
import time
import uuid

from release import ROOT, compile_tool, make_feed, output, require, run, sparkle


class Handler(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *args):
        pass


def check(source):
    tools = sparkle()
    tool = compile_tool(tools)
    identity, keychain = require('IOS_DISTRIBUTION_IDENTITY'), require('RELEASE_KEYCHAIN')
    with tempfile.TemporaryDirectory(prefix='ab-test-update-', dir=Path.home() / 'Library/Caches') as temp:
        root = Path(temp)
        public = root / 'feed'
        public.mkdir()
        server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), partial(Handler, directory=str(public)))
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            url = f'http://127.0.0.1:{server.server_port}'
            seed = root / 'update.seed'
            seed.write_text(base64.b64encode(os.urandom(32)).decode())
            seed.chmod(0o600)
            public_key = output(tool, 'public-key', seed)
            bundle = 'dev.applebridge.r1test.' + uuid.uuid4().hex
            def sign(app):
                run('codesign', '--force', '--deep', '--sign', identity, '--keychain', keychain, '--options', 'runtime', '--timestamp', app, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                run('codesign', '--verify', '--deep', '--strict', app)
            def prepare(app, build):
                run('ditto', source, app)
                info_path = app / 'Contents/Info.plist'
                info = plistlib.loads(info_path.read_bytes())
                info.update(CFBundleIdentifier=bundle, CFBundleVersion=build, SUFeedURL=url + '/appcast.xml', SUPublicEDKey=public_key, SUEnableAutomaticChecks=False, SUAutomaticallyUpdate=False, NSAppTransportSecurity={'NSAllowsLocalNetworking': True, 'NSAllowsArbitraryLoads': True})
                info_path.write_bytes(plistlib.dumps(info))
                sign(app)
            old = root / 'installed/Apple Bridge.app'
            new = root / 'new/Apple Bridge.app'
            prepare(old, '2')
            prepare(new, '3')
            archive = public / 'update.zip'
            run('ditto', '-c', '-k', '--keepParent', new, archive)
            signature = output(tools / 'bin/sign_update', '--ed-key-file', seed, '-p', archive)
            feed = public / 'appcast.xml'
            make_feed(feed, '3', '0.1.2', url.replace('http:', 'https:') + '/update.zip', archive, signature)
            feed.write_text(feed.read_text().replace('https://127.0.0.1:', 'http://127.0.0.1:'))
            run(tools / 'bin/sign_update', '--ed-key-file', seed, feed, stdout=subprocess.DEVNULL)
            driver = root / 'UpdateDriver.app'
            (driver / 'Contents/MacOS').mkdir(parents=True)
            run('ditto', tools / 'Sparkle.framework', driver / 'Contents/Frameworks/Sparkle.framework')
            (driver / 'Contents/Info.plist').write_bytes(plistlib.dumps(dict(CFBundleIdentifier=bundle + '.driver', CFBundleExecutable='update-check', CFBundlePackageType='APPL', CFBundleVersion='1', CFBundleName='[ab-test] Update Driver', NSAppTransportSecurity={'NSAllowsLocalNetworking': True, 'NSAllowsArbitraryLoads': True})))
            run('clang', '-fobjc-arc', '-Wall', '-Werror', '-F', tools, '-framework', 'Sparkle', '-framework', 'AppKit', '-rpath', '@executable_path/../Frameworks', ROOT / 'packaging/release/UpdateCheck.m', '-o', driver / 'Contents/MacOS/update-check')
            sign(driver)
            register = '/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister'
            run(register, '-f', old, driver)
            run(driver / 'Contents/MacOS/update-check', old, timeout=150)
            for attempt in range(100):
                info_path = old / 'Contents/Info.plist'
                if info_path.exists() and plistlib.loads(info_path.read_bytes())['CFBundleVersion'] == '3':
                    break
                time.sleep(0.2)
            else:
                raise ValueError('Sparkle did not replace the previous build.')
            run('codesign', '--verify', '--deep', '--strict', old)
            print('R1-PREVIOUS-VERSION-UPDATE passed=true')
        finally:
            if 'register' in locals():
                run(register, '-u', old, driver)
            server.shutdown()
            server.server_close()
            processes = subprocess.check_output(['ps', '-axo', 'pid=,comm='], text=True)
            for line in processes.splitlines():
                fields = line.strip().split(None, 1)
                if len(fields) == 2 and fields[1].startswith(str(root) + '/'):
                    try:
                        os.kill(int(fields[0]), signal.SIGTERM)
                    except ProcessLookupError:
                        pass
            if 'bundle' in locals():
                for domain in [bundle, bundle + '.driver']:
                    subprocess.run(['defaults', 'delete', domain], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                    (Path.home() / 'Library/Preferences' / (domain + '.plist')).unlink(missing_ok=True)
                    shutil.rmtree(Path.home() / 'Library/Caches' / domain, ignore_errors=True)
                    shutil.rmtree(Path.home() / 'Library/Saved Application State' / (domain + '.savedState'), ignore_errors=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description='Exercise the real Sparkle downloader and installer on a private temporary copy; no app launch or login registration.')
    parser.add_argument('--app', required=True)
    check(Path(parser.parse_args().app).resolve())
