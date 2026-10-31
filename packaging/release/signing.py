#!/usr/bin/env python3
import argparse
import base64
import json
import os
from pathlib import Path
import secrets
import shlex
import stat
import subprocess
import sys

DIRECTORY = Path.home() / '.config/apple-bridge-release'
KEYCHAIN = DIRECTORY / 'ab-release.keychain-db'
PASSWORD = DIRECTORY / 'keychain.password'


def command(*args):
    environment = dict(os.environ, ASC_BYPASS_KEYCHAIN='1') if args[0] == 'asc' else None
    try:
        result = subprocess.run([str(a) for a in args], capture_output=True, text=True, stdin=subprocess.DEVNULL, env=environment, timeout=120)
    except subprocess.TimeoutExpired:
        raise ValueError(f'{Path(args[0]).name} {args[1]} timed out. Retry release setup after checking the service.') from None
    if result.returncode:
        if args[0] == 'asc':
            detail = result.stderr.strip()
            raise ValueError(detail or 'App Store Connect rejected signing setup. Check the API key permissions.')
        raise ValueError(f'{Path(args[0]).name} {args[1]} failed. Inspect the private release setup before retrying.')
    return result.stdout.strip()


def private_file(path, data):
    with open(path, 'x', opener=lambda name, flags: os.open(name, flags, 0o600)) as file:
        file.write(data)


def prepare_keychain():
    DIRECTORY.mkdir(mode=0o700, parents=True, exist_ok=True)
    if DIRECTORY.is_symlink() or DIRECTORY.stat().st_mode & 0o077:
        raise ValueError('The release credential directory must be private (mode 700) and not a symlink.')
    if not PASSWORD.exists():
        if KEYCHAIN.exists():
            raise ValueError('Restore the password file for the existing release keychain; it will not be replaced.')
        private_file(PASSWORD, secrets.token_hex(32))
    if PASSWORD.is_symlink() or not stat.S_ISREG(PASSWORD.stat().st_mode) or stat.S_IMODE(PASSWORD.stat().st_mode) != 0o600:
        raise ValueError('The release keychain password must be a regular mode-600 file.')
    password = PASSWORD.read_text().strip()
    if not password:
        raise ValueError('The release keychain password file is empty. Restore it before retrying.')
    if not KEYCHAIN.exists():
        command('security', 'create-keychain', '-p', password, KEYCHAIN)
    command('security', 'unlock-keychain', '-p', password, KEYCHAIN)
    command('security', 'set-keychain-settings', KEYCHAIN)
    return password


def provision(kind, certificate=None):
    password = prepare_keychain()
    stem = DIRECTORY / ('developer-id' if kind == 'DEVELOPER_ID_APPLICATION' else 'ios-distribution')
    key, csr, cert, record = [stem.with_suffix(suffix) for suffix in ['.key', '.csr', '.der', '.json']]
    if record.exists() and not key.exists():
        raise ValueError('Restore the private key for the existing certificate before retrying release setup.')
    if not key.exists():
        command('openssl', 'genrsa', '-out', key, '2048')
        key.chmod(0o600)
    if not csr.exists():
        command('openssl', 'req', '-new', '-sha256', '-key', key, '-out', csr, '-subj', '/CN=Apple Bridge Release')
    if not record.exists():
        if certificate:
            command('openssl', 'x509', '-inform', 'DER', '-in', certificate, '-noout')
            response = {'data': {'attributes': {'certificateContent': base64.b64encode(Path(certificate).expanduser().read_bytes()).decode()}}}
        else:
            response = json.loads(command('asc', 'certificates', 'create', '--certificate-type', kind, '--csr', csr))
        private_file(record, json.dumps(response))
    response = json.loads(record.read_text())['data']
    cert.write_bytes(base64.b64decode(response['attributes']['certificateContent'], validate=True))
    cert.chmod(0o600)
    identities = command('security', 'find-identity', '-v', '-p', 'codesigning', KEYCHAIN)
    fingerprint = command('openssl', 'x509', '-inform', 'DER', '-in', cert, '-noout', '-fingerprint', '-sha1').split('=')[-1].replace(':', '')
    if fingerprint not in identities:
        command('security', 'import', key, '-k', KEYCHAIN, '-T', '/usr/bin/codesign')
        command('security', 'import', cert, '-k', KEYCHAIN, '-T', '/usr/bin/codesign')
    command('security', 'set-key-partition-list', '-S', 'apple-tool:,apple:,codesign:', '-s', '-k', password, KEYCHAIN)
    current = shlex.split(command('security', 'list-keychains', '-d', 'user'))
    if str(KEYCHAIN) not in current:
        command('security', 'list-keychains', '-d', 'user', '-s', *current, KEYCHAIN)
    identities = command('security', 'find-identity', '-v', '-p', 'codesigning', KEYCHAIN)
    if fingerprint not in identities:
        raise ValueError('The release certificate is not a valid signing identity. Check its Apple intermediate certificate.')
    variable = 'DEVELOPER_ID_APPLICATION' if kind == 'DEVELOPER_ID_APPLICATION' else 'IOS_DISTRIBUTION_IDENTITY'
    environment = {variable: fingerprint, 'RELEASE_KEYCHAIN': str(KEYCHAIN)}
    config = DIRECTORY / 'signing.json'
    existing = json.loads(config.read_text()) if config.exists() else {}
    existing.update(environment)
    config.write_text(json.dumps(existing, indent=2) + '\n')
    config.chmod(0o600)
    print(f'R1-SIGNING-{kind} passed=true')


def main():
    parser = argparse.ArgumentParser(description='Provision a private release keychain without using the login keychain.')
    parser.add_argument('--certificate-type', choices=['DEVELOPER_ID_APPLICATION', 'IOS_DISTRIBUTION', 'DISTRIBUTION'], default='DEVELOPER_ID_APPLICATION')
    parser.add_argument('--certificate', type=lambda value: Path(value).expanduser().resolve(), help='Import an Apple-issued DER certificate for the saved CSR instead of requesting one through ASC')
    args = parser.parse_args()
    os.umask(0o077)
    try:
        provision(args.certificate_type, args.certificate)
    except (ValueError, OSError, KeyError) as error:
        print(f'Release signing preparation failed: {error}', file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main())
