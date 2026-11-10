import argparse
import datetime
import json
import os
import pathlib
import subprocess
import sys
import time


def control(socket, route, body):
    out = subprocess.run(['curl', '-s', '--max-time', '120', '--unix-socket', str(socket), '-H', 'Content-Type: application/json', '--data-binary', '@-', 'http://sim' + route], input=json.dumps(body).encode(), capture_output=True, check=True)
    return out.stdout.decode()


def fixtures():
    now = datetime.datetime.now(datetime.timezone.utc)
    local = now.astimezone()
    day = lambda n: (local - datetime.timedelta(days=n)).strftime('%Y-%m-%d')
    iso = lambda d: d.strftime('%Y-%m-%dT%H:%M:%SZ')
    ref = lambda c, k: {'$ref': {'collection': c, 'native_key': k}}

    def record(collection, key, data):
        return {'collection': collection, 'native_key': key, 'kind': 'upsert', 'observed_at': iso(now), 'data': data}

    def message(key, chat, mine, sender, text, at):
        data = {'chat_id': ref('messages.chat', chat), 'decode_status': 'decoded', 'edit_history': [], 'error_code': 0, 'is_from_me': mine, 'kind': 'message', 'parts': [{'index': 0, 'kind': 'text', 'text': text}], 'retracted_parts': [], 'sent_at': at, 'text': text}
        if sender:
            data['sender_handle_id'] = ref('messages.handle', sender)
        return record('messages.message', key, data)

    def event(key, calendar, title, start, end, location=None):
        data = {'calendar_id': ref('calendar.calendar', calendar), 'title': title, 'start': iso(start), 'end': iso(end), 'all_day': False, 'alarms': [], 'attendees': [], 'attachments': [], 'availability': 0, 'status': 1, 'is_exception': False, 'recurrence': []}
        if location:
            data['location'] = location
        return record('calendar.event', key, data)

    dentist = local.replace(hour=15, minute=0, second=0, microsecond=0).astimezone(datetime.timezone.utc)
    early = (local - datetime.timedelta(hours=3)).strftime('%Y-%m-%dT%H:%M:%SZ')
    rows = [
        record('contacts.contact', 'x-ana-novak', {'given_name': 'Ana', 'middle_name': '', 'family_name': 'Novak', 'phones': [{'label': 'mobile', 'value': '+1 555 0201'}], 'emails': [], 'writable': True, 'container_id': 'demo-synthetic'}),
        record('contacts.contact', 'x-ana-kovac', {'given_name': 'Ana', 'family_name': 'Kovač', 'phones': [{'label': 'mobile', 'value': '+1 555 0202'}], 'emails': [], 'writable': True, 'container_id': 'demo-synthetic'}),
        record('contacts.contact', 'x-marko', {'given_name': 'Marko', 'family_name': 'Horvat', 'phones': [{'label': 'mobile', 'value': '+1 555 0203'}], 'emails': [{'label': 'home', 'value': 'marko@example.com'}], 'writable': True, 'container_id': 'demo-synthetic'}),
        record('messages.handle', 'x-h-ana-novak', {'address': '+15550201', 'service': 'iMessage'}),
        record('messages.handle', 'x-h-ana-kovac', {'address': '+15550202', 'service': 'iMessage'}),
        record('messages.handle', 'x-h-marko', {'address': '+15550203', 'service': 'iMessage'}),
        record('messages.chat', 'x-chat-marko', {'capabilities': [], 'display_name': None, 'filter': 'known', 'is_archived': False, 'participants': [ref('messages.handle', 'x-h-marko')], 'service': 'iMessage', 'style': 'direct', 'read_receipts': 'default'}),
        record('messages.chat', 'x-chat-trip', {'capabilities': [], 'display_name': 'Trip planning', 'filter': 'known', 'is_archived': False, 'participants': [ref('messages.handle', 'x-h-ana-novak'), ref('messages.handle', 'x-h-marko')], 'service': 'iMessage', 'style': 'group', 'read_receipts': 'default'}),
        message('x-m1', 'x-chat-marko', False, 'x-h-marko', 'Did you see the sim results?', early),
        message('x-m2', 'x-chat-marko', True, 'message-self', 'Yes, the unicode bug is fixed', iso(now - datetime.timedelta(hours=2))),
        message('x-m3', 'x-chat-trip', False, 'x-h-ana-novak', '  Ferry leaves at 9, bring snacks  \n', early),
        record('messages.attachment', 'x-att-1', {'blob': {'digest': '6b0d6b7976bcba19382169f9bf1293b12400bcf1143b99a86d9b705c6bce2d3e', 'state': 'present'}, 'filename': 'ferry-map.png', 'message_id': ref('messages.message', 'x-m3'), 'mime_type': 'image/png', 'size': 31761, 'uti': 'image/png'}),
        event('x-dentist', 'demo-cal-personal', 'Dentist', dentist, dentist + datetime.timedelta(hours=1), 'Main Street 1'),
        record('reminders.list', 'x-bridge-test', {'title': 'Bridge Test', 'writable': True, 'account': 'demo-account', 'is_group': False, 'sections': [], 'display_order': 9}),
        record('reminders.reminder', 'x-plumber', {'list_id': ref('reminders.list', 'x-bridge-test'), 'title': 'Call the plumber', 'completed': False, 'priority': 0, 'notes': None, 'flagged': False}),
        record('notes.folder', 'x-notes-folder', {'title': 'Notes', 'account': 'iCloud'}),
        record('notes.note', 'x-phone-note', {'folder_id': ref('notes.folder', 'x-notes-folder'), 'title': 'Ideas from the train', 'plain_text': 'Ideas from the train\nAsk Ana about the ferry\nBook the dentist', 'modified_at': iso(now - datetime.timedelta(hours=2)), 'created_at': iso(now - datetime.timedelta(hours=3))}),
        record('usage.device', 'x-mac', {'name': 'Synthetic MacBook', 'platform': 'mac', 'build': 'sim', 'is_local': True, 'latest_event_at': iso(now), 'provenance': 'biome.app_infocus', 'decoder_version': 'infocus-segb-1'}),
        record('usage.device', 'x-iphone', {'name': 'Synthetic iPhone', 'platform': 'iphone', 'build': 'sim', 'is_local': False, 'latest_event_at': iso(now), 'provenance': 'biome.app_infocus', 'decoder_version': 'infocus-segb-1'}),
    ]
    usage = [('x-mac', 'com.apple.Safari', 5400), ('x-mac', 'com.microsoft.VSCode', 9000), ('x-mac', 'com.apple.MobileSMS', 1200), ('x-iphone', 'com.burbn.instagram', 3600), ('x-iphone', 'com.apple.MobileSMS', 1800), ('x-iphone', 'com.google.ios.youtube', 2400)]
    for n, (device, bundle, seconds) in enumerate(usage):
        for back in (1, 2):
            rows.append(record('usage.app_day', f'x-u{n}-{back}', {'device_id': ref('usage.device', device), 'day': day(back), 'bundle_id': bundle, 'seconds_estimated': seconds * back, 'clipped_seconds': 0, 'provenance': 'biome.app_infocus', 'decoder_version': 'infocus-segb-1', 'uncertainty': 'focus estimates, not active interaction'}))
    for i in range(12):
        start = (local + datetime.timedelta(days=i % 9)).replace(hour=10, minute=0, second=0, microsecond=0).astimezone(datetime.timezone.utc)
        rows.append(event(f'x-noise-{i}', 'demo-cal-work', f'searchcapnoise sync {i}', start, start + datetime.timedelta(hours=1)))
    rows.append(record('notes.note', 'x-noise-note', {'folder_id': ref('notes.folder', 'x-notes-folder'), 'title': 'searchcapnoise note', 'plain_text': 'searchcapnoise note body', 'modified_at': iso(now - datetime.timedelta(days=3))}))
    return rows


def agent(bridge, home, name, level, scopes):
    out = subprocess.run([str(bridge), 'agents', 'create', '--name', name, '--level', level, '--scope', ','.join(scopes), '--home', str(home), '--json'], capture_output=True, text=True, check=True)
    return json.loads(out.stdout)['token']


def main():
    parser = argparse.ArgumentParser(description='Start a synthetic Apple Bridge world for sim_suite.py: a gateway, SimApple seeded with the demo dataset and the suite fixtures, and agent tokens.')
    parser.add_argument('--bin', required=True, help='directory with bridge (built with lab-queries), lab-simapple and lab-demo')
    parser.add_argument('--home', required=True, help='new private directory for the world')
    parser.add_argument('--port', type=int, default=17890)
    options = parser.parse_args()
    binaries = pathlib.Path(options.bin)
    home = pathlib.Path(options.home)
    home.mkdir(mode=0o700, parents=True, exist_ok=False)
    environment = dict(os.environ, APPLE_BRIDGE_SECRET_STORE='file')
    gateway = home / 'gateway'
    with (home / 'gateway.log').open('w') as log:
        process = subprocess.Popen([str(binaries / 'bridge'), 'start', '--role', 'gateway', '--home', str(gateway), '--peer-addr', '127.0.0.1:0', '--api-addr', f'127.0.0.1:{options.port}'], stdout=log, stderr=log, env=environment, start_new_session=True)
    (home / 'gateway.pid').write_text(str(process.pid))
    for _ in range(300):
        if (gateway / 'runtime.json').exists():
            break
        time.sleep(0.1)
    socket = json.loads((gateway / 'runtime.json').read_text())['socket']
    sim = home / 'sim.sock'
    with (home / 'simapple.log').open('w') as log:
        process = subprocess.Popen([str(binaries / 'lab-simapple'), 'simapple', '--socket', socket, '--secret-file', str(gateway / 'apple-secret'), '--control', str(sim)], stdout=log, stderr=log, start_new_session=True)
    (home / 'sim.pid').write_text(str(process.pid))
    for _ in range(300):
        if sim.exists():
            break
        time.sleep(0.1)
    demo = json.loads(subprocess.run([str(binaries / 'lab-demo')], capture_output=True, text=True, check=True).stdout)
    control(sim, '/dataset', {'records': demo['records'] + fixtures()})
    catalog = json.loads((pathlib.Path(__file__).resolve().parents[2] / 'contracts/golden/catalog.json').read_text())
    operations = catalog['operations'] if isinstance(catalog, dict) else catalog
    scopes = sorted({o['scope'] for o in operations if o['scope'] and not o['scope'].startswith('system')} | {f'{s}:read' for s in ('usage', 'safari', 'voicemail', 'phone', 'voice_memos')})
    for level in ('full', 'approve', 'read'):
        (home / f'{level}.tok').write_text(agent(binaries / 'bridge', gateway, f'suite-{level}', level, scopes))
    (home / 'grader.tok').write_text(agent(binaries / 'bridge', gateway, 'suite-grader', 'full', scopes))
    for token in home.glob('*.tok'):
        token.chmod(0o600)
    time.sleep(4)
    print(json.dumps({'url': f'http://127.0.0.1:{options.port}/mcp', 'home': str(home), 'sim_socket': str(sim)}))
    return 0


if __name__ == '__main__':
    sys.exit(main())
