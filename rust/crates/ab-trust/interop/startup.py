import base64
import json
import pathlib
import subprocess
import sys
import tempfile

root = pathlib.Path(__file__).resolve().parents[4]
fixture = json.loads((root / 'contracts/golden/passkey.json').read_text())
with tempfile.TemporaryDirectory(prefix='ab-gd1-refusal-') as directory:
    home = pathlib.Path(directory)
    key = dict(id='synthetic', rp_id='other.example', created_at='2026-01-01T00:00:00Z', credential=fixture['stored_credential'])
    key['credential']['publicKey'] = base64.b64encode(bytes.fromhex('a2010303390100')).decode()
    path = home / 'credentials.json'
    original = json.dumps(dict(passkeys=[key])).encode()
    path.write_bytes(original)
    path.chmod(0o600)
    before = sorted(p.name for p in home.iterdir())
    for verb in ['start', 'doctor']:
        result = subprocess.run([sys.argv[1], verb, '--role', 'gateway', '--home', str(home)], capture_output=True, text=True, timeout=10)
        assert result.returncode != 0, result
        assert 'unsupported stored passkey algorithm -257' in result.stderr, result.stderr
        assert 're-enrol' in result.stderr, result.stderr
        assert path.read_bytes() == original
        assert sorted(p.name for p in home.iterdir()) == before
    print(json.dumps(dict(startup='refused with re-enrol hint', doctor='refused with re-enrol hint', credential_rp='other.example', home_unchanged=True)))
