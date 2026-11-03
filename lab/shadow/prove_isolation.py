import argparse
import fcntl
import json
import os
from pathlib import Path
import socket
import struct
import subprocess
import sys

from isolation import loopback_only, require_home, require_isolation
from shadow import private_directory


def inside(args):
    if not loopback_only():
        raise ValueError('namespace has external interfaces')
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as control:
        fcntl.ioctl(control, 0x8914, struct.pack('16sh', b'lo', 1))
    if args.privileged_namespace:
        os.setgroups([])
        os.setgid(args.gid)
        os.setuid(args.uid)
    require_isolation()
    escaped = False
    try:
        with socket.create_connection(('127.0.0.1', args.host_port), timeout=.5):
            escaped = True
    except OSError:
        pass
    with socket.socket() as listener:
        listener.bind(('127.0.0.1', 0))
        listener.listen()
        with socket.create_connection(listener.getsockname(), timeout=1):
            accepted, _ = listener.accept()
            accepted.close()
    root = private_directory(args.output)
    home = root / 'remote-home'
    (home / 'remote').mkdir(parents=True)
    marker = home / 'remote' / 'config.json'
    raw = b'{"enabled":true,"backend":"system"}'
    marker.write_bytes(raw)
    refused = False
    try:
        require_home(home)
    except ValueError:
        refused = True
    result = {'host_loopback_unreachable': not escaped, 'private_loopback_usable': True, 'remote_adapter_refused': refused, 'starting_home_unchanged': marker.read_bytes() == raw}
    print(json.dumps(result, sort_keys=True))
    return int(not all(result.values()))


def main():
    os.umask(0o077)
    parser = argparse.ArgumentParser()
    parser.add_argument('--output', required=True)
    parser.add_argument('--privileged-namespace', action='store_true')
    parser.add_argument('--inside', action='store_true')
    parser.add_argument('--host-port', type=int)
    parser.add_argument('--uid', type=int)
    parser.add_argument('--gid', type=int)
    args = parser.parse_args()
    if args.inside:
        return inside(args)
    root = private_directory(args.output)
    if list(root.iterdir()):
        raise ValueError('isolation proof output must be empty')
    with socket.socket() as sentinel:
        sentinel.bind(('127.0.0.1', 0))
        sentinel.listen()
        prefix = ['sudo', '-n', 'unshare', '--net'] if args.privileged_namespace else ['unshare', '--user', '--map-root-user', '--net']
        command = [*prefix, sys.executable, str(Path(__file__).resolve()), '--inside', '--output', str(root), '--host-port', str(sentinel.getsockname()[1]), '--uid', str(os.getuid()), '--gid', str(os.getgid())]
        if args.privileged_namespace:
            command.append('--privileged-namespace')
        return subprocess.run(command).returncode


if __name__ == '__main__':
    sys.exit(main())
