import json
import fcntl
import os
from pathlib import Path
import socket
import struct
import subprocess
import sys


def loopback_only():
    return sys.platform == 'linux' and {name for _, name in socket.if_nameindex()} == {'lo'}


def require_isolation():
    if loopback_only():
        return
    if sys.platform != 'linux':
        raise ValueError('replay requires a Linux network namespace containing only loopback')
    command = ['unshare', '--user', '--map-root-user', '--net', sys.executable, str(Path(__file__).resolve()), *sys.argv]
    try:
        result = subprocess.run(command)
    except OSError as error:
        raise ValueError('replay isolation unavailable; no gateway launched') from error
    raise SystemExit(result.returncode)


def inert_status(entry):
    if entry.name != 'status.json' or entry.is_symlink() or not entry.is_file():
        return False
    try:
        return json.loads(entry.read_text()).get('state') == 'off'
    except (OSError, ValueError, AttributeError):
        return False


def require_home(home):
    remote = Path(home) / 'remote'
    if remote.exists() and any(not inert_status(entry) for entry in remote.iterdir()):
        raise ValueError('replay refuses saved remote adapters: host Unix sockets are outside network isolation')


def main():
    if not loopback_only():
        raise ValueError('private network namespace unavailable')
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as control:
        request = struct.pack('16sh', b'lo', 1)
        fcntl.ioctl(control, 0x8914, request)
    os.execv(sys.executable, [sys.executable, *sys.argv[1:]])


if __name__ == '__main__':
    main()
