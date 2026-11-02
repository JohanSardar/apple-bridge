import pathlib
import subprocess
import sys
import tempfile

oracle, rust = sys.argv[1:]
with tempfile.TemporaryDirectory(prefix='ab-synthetic-tls-') as directory:
    fingerprints = dict(line.split() for line in subprocess.check_output([oracle, 'tls', 'generate', directory], text=True).splitlines())
    count = 0
    for server, client, server_name, client_name in [(oracle, rust, 'go', 'rust'), (rust, oracle, 'rust', 'go')]:
        for wrong_server, wrong_client in [(False, False), (True, False), (False, True)]:
            server_pin = '0'*64 if wrong_client else fingerprints[client_name]
            client_pin = '0'*64 if wrong_server else fingerprints[server_name]
            with subprocess.Popen([server, 'tls', 'server', directory, server_pin], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True) as process:
                port = process.stdout.readline().strip()
                result = subprocess.run([client, 'tls', 'client', directory, client_pin, port], capture_output=True, timeout=15)
                server_result = process.wait(timeout=15)
                expect_success = not wrong_server and not wrong_client
                if (result.returncode == 0 and server_result == 0) != expect_success:
                    raise RuntimeError('mTLS matrix mismatch')
                count += 1
    for server, client, server_name, client_name in [(oracle, rust, 'go', 'rust'), (rust, oracle, 'rust', 'go')]:
        for expired in [server_name, client_name]:
            saved = {}
            for suffix in ['der', 'key']:
                path = pathlib.Path(directory) / f'{expired}.{suffix}'
                saved[suffix] = path.read_bytes()
                path.write_bytes((pathlib.Path(directory) / f'{expired}-expired.{suffix}').read_bytes())
            pins = dict(fingerprints)
            pins[expired] = fingerprints[expired+'-expired']
            with subprocess.Popen([server, 'tls', 'server', directory, pins[client_name]], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True) as process:
                port = process.stdout.readline().strip()
                result = subprocess.run([client, 'tls', 'client', directory, pins[server_name], port], capture_output=True, timeout=15)
                server_result = process.wait(timeout=15)
                if result.returncode == 0 and server_result == 0:
                    raise RuntimeError('expired certificate accepted')
            for suffix, content in saved.items():
                (pathlib.Path(directory) / f'{expired}.{suffix}').write_bytes(content)
            count += 1
    print(f'mtls cases={count} mismatches=0 positive=2 wrong-pin-rejected=4 expired-rejected=4')
