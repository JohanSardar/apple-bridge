import argparse
import json
from pathlib import Path
import subprocess

HARNESS_ONLY = {'credential_backend_override_restored'}
BLOB_SCENARIOS = ['CORE-BLOB-CAP', 'CORE-BLOB-CORRUPT', 'CORE-BLOB-DUPLICATE-REFERENCE',
                  'CORE-BLOB-GATEWAY-RETRY', 'CORE-BLOB-GC', 'CORE-BLOB-INTERRUPT',
                  'CORE-BLOB-NO-DIGEST-AUTH', 'CORE-BLOB-OFFLOADED', 'CORE-BLOB-REFERENCE-INDEX',
                  'CORE-BLOB-SLOW-READ', 'CORE-BLOB-STAGE-DURING-TRANSFER',
                  'CORE-BLOB-STAGE-SAME-DIGEST']
JOURNEY_SCENARIOS = ['CORE-CMD-IDEMPOTENT', 'CORE-CMD-ERA', 'CORE-CMD-LOST-RESPONSE', 'CORE-CMD-CONFLICT',
                     'CORE-CMD-READ-YOUR-WRITE', 'CORE-CMD-CRASH-AMBIGUOUS', 'CORE-CMD-EXPIRY',
                     'CORE-CMD-CLOCK-JUMP', 'CORE-CMD-OFFLINE', *BLOB_SCENARIOS]
JOURNEY_RUST_ONLY = {'world_started'}
CORE_SCENARIOS = ['CORE-RESUME', 'CORE-STALE-FENCE', 'LAB-MALFORMED-SCENARIO', 'LAB-SSE',
                  'LAB-TRUTH-MULTIPLICITY']
IMPORT_SCENARIOS = ['messages/MSG-HISTORY-IMPORT', 'contacts/CON-IMPORT', 'notes/NOTE-IMPORT', 'usage/USE-IMPORT']
SHORTCUT_SCENARIOS = ['shortcuts/FILES-TRANSFORM', 'shortcuts/SC-ACTIONS', 'shortcuts/SC-CAPTURE', 'shortcuts/SC-RUN-REAL',
                      'shortcuts/CLK-SIM', 'shortcuts/CLK-ALARM-REAL', 'shortcuts/CLK-TIMER-REAL']
CONTACTS_SCENARIOS = ['contacts/CON-CRUD', 'contacts/CON-CONFLICT', 'contacts/CON-DROP-EVERYTHING',
                      'contacts/CON-EXTERNAL', 'contacts/CON-GROUPS', 'contacts/CON-HANDLE-LINK',
                      'contacts/CON-IMAGE', 'contacts/CON-PERMISSION-LOSS',
                      'contacts/CON-READONLY-CONTAINER', 'contacts/CON-RESOLVE-RESTART',
                      'contacts/CON-SEARCH', 'contacts/CON-TOKEN-CRASH',
                      'contacts/CON-RECIPIENT-LOOKUP']
NOTES_SCENARIOS = ['notes/NOTE-CRUD', 'notes/NOTE-RICH-REJECT', 'notes/NOTE-EXTERNAL', 'notes/NOTE-EXTRAS',
                   'notes/NOTE-LIST', 'notes/NOTE-EDIT-PARITY']
PIM_NATIVE_SCENARIOS = ['notes/NOTE-DECODE', 'mail/MAIL-CAPTURE', 'mail/MAIL-BODY', 'mail/MAIL-COMMANDS',
                        'mail/MAIL-IDLE', 'mail/MAIL-LIVE', 'mail/MAIL-NO-FALSE-DELETE']
MEDIA_SCENARIOS = ['web/WEB-MEDIA-VARIANTS', 'messages/MSG-MEDIA-CLEANUP']
WEB_RM_SCENARIOS = ['web/WEB-RM-MEDIA-COLLECTION', 'web/WEB-RM-SEARCH', 'web/WEB-RM-THREAD-ORIGINATOR',
                    'web/WEB-RM-THREAD-ORIGINATOR-PERF', 'web/WEB-RM-REMINDERS-DEMO']
NATIVE_SCENARIOS = ['packaging/Q1-SETUP', 'packaging/R1-RELEASE-SIM', 'files/FILE-IDLE', 'files/FILE-CAPTURE',
                    'safari/SAF-CAPTURE', 'safari/SAF-IDLE']
MESSAGES_A_SCENARIOS = ['messages/MSG-DB-REPLACED', 'messages/MSG-PERMISSION-LOSS', 'messages/MSG-INCOMING',
                        'messages/MSG-DECODE']
USAGE_SCENARIOS = ['usage/USE-LOCK-CLIP', 'usage/USE-READONLY', 'usage/USE-REMOTE-LAG']
DARWIN_USAGE_SCENARIOS = ['usage/USE-SEGB-DECODE', 'usage/USE-ACTION-PROBE']
CALLS_SCENARIOS = ['phone/CALL-CAPTURE', 'phone/VM-CAPTURE', 'phone/VMEMO-CAPTURE', 'phone/VMEMO-LATE-ASSETS', 'phone/CALL-IDLE']
MESSAGES_A_COMMAND_SCENARIOS = ['messages/MSG-CAPABILITY-MATRIX', 'messages/MSG-COMMAND-OBSERVED-IDS',
                                'messages/MSG-COMMAND-PARTS', 'messages/MSG-DELETE', 'messages/MSG-EDIT',
                                'messages/MSG-FAILED-OBSERVED-IDS', 'messages/MSG-MARK-READ',
                                'messages/MSG-OWNER-TOKEN-SELFCHAT', 'messages/MSG-DELETE-REAL',
                                'messages/MSG-DECODE-BALLOON', 'messages/MSG-DECODE-FORMATTING']
HELPER_SCENARIOS = ['helper/HELPER-IPC-RESPONSIVE', 'helper/SESSION-PROXY-BYTES', 'helper/SIRI-INTENTS']
IPHONE_SCENARIOS = ['iphone/APP-ALARM-SIM', 'iphone/APP-SHELL-SIM', 'iphone/APP-PAIR-SIM', 'iphone/APP-DEVICE-SIM',
                    'iphone/APP-MOTION-SIM', 'iphone/APP-HEALTH-WRITE-SIM', 'iphone/APP-SCREENTIME-SIM',
                    'iphone/APP-HEALTH-SIM', 'iphone/APP-LOC-SIM', 'iphone/APP-LIVE-SIM', 'iphone/APP-APPROVE-SIM',
                    'iphone/APP-CMD-SIM', 'iphone/APP-NOTIFY-SIM', 'iphone/APP-CAPTURE-SIM']
RELAY_SCENARIOS = ['RELAY-ENROLL', 'RELAY-PASSTHROUGH', 'RELAY-LIMITS', 'RELAY-ROTATE-GATED',
                   'RELAY-CT-INDEPENDENT', 'RELAY-CAA-ACCOUNT']
CONNECTOR_SCENARIOS = ['AUTH-SCOPES-METADATA', 'CONN-CHATGPT', 'CONN-CLAUDE-AI', 'RMT-PRIVATE-CONNECT']
REMINDER_SCENARIOS = ['REM-CONFLICT', 'REM-EXTERNAL-RACE', 'REM-NATIVE-LIST-RACE', 'REM-OPERATION-SHAPES',
                      'REM-READONLY-LIST']
REMINDER_NATIVE_SCENARIOS = ['core/REM-CRUD', 'reminders/REM-CRUD', 'reminders/REM-CREATE-EXTERNAL',
                             'reminders/REM-LIST-MANAGE', 'reminders/REM-LIST-CREATE', 'reminders/REM-RECURRING',
                             'reminders/REM-PERMISSION-LOSS']
MAC_SCENARIOS = ['mac/MAC-ACTIONS', 'mac/MAC-COMMANDS', 'mac/MAC-CAPTURE', 'mac/MAC-IDLE',
                 'mac/MAC-EXTRAS-SIM', 'mac/IWORK-REAL']
WEB_SCENARIOS = ['web/WEB-CON-LIST', 'web/WEB-MEDIA-CAP']
NATIVE_MEDIA_SCENARIOS = ['web/NATIVE-MEDIA-EXPORT-SAFETY', 'web/NATIVE-MEDIA-POSTERS', 'web/NATIVE-MEDIA-YIELD', 'web/WEB-REM-PARITY']
MESSAGES_B_SCENARIOS = [f'messages/MSG-{name}' for name in [
    'REACT', 'READSTATE-REAL', 'RECEIPTS', 'REPLY', 'RESEND', 'SELFTEST', 'SEND-FILES', 'SEND-GHOST',
    'SEND-IDENTICAL-MANUAL', 'SEND-NEW-HANDLE', 'SEND-SELF-EMAIL', 'SEND-SELF-TEXT', 'UI-INTERFERENCE', 'UNSEND']]
MESSAGES_B_NATIVE = ['messages/MSG-WATCHER-REARM']
MAPS_SCENARIOS = ['maps/MAP-SIM', 'maps/LOC-SIM', 'files/FILE-COMMANDS', 'files/FILE-CRUD-REAL',
                  'files/FILE-OFFLOAD-REAL']
CALENDAR_SCENARIOS = ['calendar/' + n for n in [
    'CAL-ALLDAY', 'CAL-ALLDAY-FUTURE', 'CAL-CREATE-EXTERNAL', 'CAL-CRUD', 'CAL-DST', 'CAL-FUTURE-RULE-MISMATCH',
    'CAL-ID-CHURN', 'CAL-LARGE-CAPTURE', 'CAL-MOVED-OCCURRENCE', 'CAL-MOVED-OUTSIDE-WINDOW',
    'CAL-PERIODIC-VERIFICATION', 'CAL-PERMISSION-LOSS', 'CAL-READONLY', 'CAL-RECUR-ALL-REJECT',
    'CAL-RECUR-FUTURE', 'CAL-RECUR-THIS', 'CAL-SPLIT-ATOMIC-ALLDAY', 'CAL-SPLIT-ATOMIC-TIMED',
    'CAL-SPLIT-FRAME-CAP', 'CAL-STAMP-FIELDS', 'CAL-WINDOW-SLIDE']]
CALENDAR_NATIVE = ['calendar/CAL-STORE-READ', 'calendar/CAL-FUTURE-COUNT-MATH', 'calendar/CAL-HINT-COVERAGE',
                   'calendar/CAL-INVALIDATED-EVENT']
VISION_SCENARIOS = ['vision/AUD-SIM', 'vision/PDF-ANNOTATE', 'vision/PDF-TEXT', 'vision/VIS-OCR']
VISION_NATIVE_SCENARIOS = ['vision/CAP-SIM']
SCENARIOS = ['CORE-QUERY-ROUNDTRIP', 'CORE-QUERY-BLOB', 'CORE-QUERY-SCOPE', 'CORE-QUERY-DEADLINE',
             *JOURNEY_SCENARIOS, *CORE_SCENARIOS, *CONTACTS_SCENARIOS, *NOTES_SCENARIOS]
FRONTDOOR_SCENARIOS = ['CORE-REST-MCP-PARITY', 'CORE-LOCALHOST-DEFENSE']
SCENARIOS += FRONTDOOR_SCENARIOS
ROBUST_SCENARIOS = ['CORE-KILL-DURING-APPLY', 'CORE-KILL-GATEWAY-CAPTURE']
SCENARIOS += ROBUST_SCENARIOS


def ignored(scenario, name):
    if name in HARNESS_ONLY or name == 'world_started':
        return True
    return scenario in JOURNEY_SCENARIOS + FRONTDOOR_SCENARIOS and name in JOURNEY_RUST_ONLY


def verdicts(path):
    rows = {}
    for line in path.read_text().splitlines():
        if line.startswith('{'):
            row = json.loads(line)
            key = row['scenario']
            while key in rows:
                key += '#'
            rows[key] = {a['name']: (a['passed'], a.get('status', ''))
                                     for a in row['assertions']
                                     if not ignored(row['scenario'], a['name'])}
    return rows


def main():
    parser = argparse.ArgumentParser()
    for name in ['families', 'go-lab', 'gateway', 'simapple', 'output']:
        parser.add_argument('--' + name, required=True)
    parser.add_argument('--scenario', action='append')
    parser.add_argument('--import', dest='imports', action='store_true')
    parser.add_argument('--messages-a', dest='messages_a', action='store_true')
    parser.add_argument('--messages-a-commands', dest='messages_a_commands', action='store_true')
    parser.add_argument('--media', dest='media', action='store_true')
    parser.add_argument('--messages-b', dest='messages_b', action='store_true')
    parser.add_argument('--messages-b-native', dest='messages_b_native', action='store_true')
    parser.add_argument('--web-rm', dest='web_rm', action='store_true')
    parser.add_argument('--native', dest='native', action='store_true')
    parser.add_argument('--iphone', dest='iphone', action='store_true')
    parser.add_argument('--maps', dest='maps', action='store_true')
    parser.add_argument('--shortcuts', dest='shortcuts', action='store_true')
    parser.add_argument('--relay', dest='relay', action='store_true')
    parser.add_argument('--go-rust-gateway', dest='go_rust_gateway', action='store_true')
    parser.add_argument('--connectors', dest='connectors', action='store_true')
    parser.add_argument('--reminders', dest='reminders', action='store_true')
    parser.add_argument('--mac', dest='mac', action='store_true')
    parser.add_argument('--usage', action='store_true')
    parser.add_argument('--darwin-usage', dest='darwin_usage', action='store_true')
    parser.add_argument('--calls', action='store_true')
    parser.add_argument('--helper', action='store_true')
    parser.add_argument('--web', dest='web', action='store_true')
    parser.add_argument('--native-media', dest='native_media', action='store_true')
    parser.add_argument('--calendar', dest='calendar', action='store_true')
    parser.add_argument('--calendar-native', dest='calendar_native', action='store_true')
    parser.add_argument('--vision', action='store_true')
    parser.add_argument('--vision-native', dest='vision_native', action='store_true')
    args = parser.parse_args()
    for flag, group in [(args.usage, USAGE_SCENARIOS), (args.darwin_usage, DARWIN_USAGE_SCENARIOS), (args.calls, CALLS_SCENARIOS), (args.helper, HELPER_SCENARIOS)]:
        if flag:
            args.scenario = (args.scenario or []) + group
    if args.shortcuts:
        args.scenario = (args.scenario or []) + SHORTCUT_SCENARIOS
    if args.messages_b:
        args.scenario = (args.scenario or []) + MESSAGES_B_SCENARIOS
    if args.messages_b_native:
        args.scenario = (args.scenario or []) + MESSAGES_B_NATIVE
    if args.imports:
        args.scenario = (args.scenario or []) + IMPORT_SCENARIOS
    if args.messages_a:
        args.scenario = (args.scenario or []) + MESSAGES_A_SCENARIOS
    if args.messages_a_commands:
        args.scenario = (args.scenario or []) + MESSAGES_A_COMMAND_SCENARIOS
    if args.media:
        args.scenario = (args.scenario or []) + MEDIA_SCENARIOS
    if args.web_rm:
        args.scenario = (args.scenario or []) + WEB_RM_SCENARIOS
    if args.native:
        args.scenario = (args.scenario or []) + NATIVE_SCENARIOS
    if args.iphone:
        args.scenario = (args.scenario or []) + IPHONE_SCENARIOS
    if args.relay:
        args.scenario = (args.scenario or []) + RELAY_SCENARIOS
    if args.connectors:
        args.scenario = (args.scenario or []) + CONNECTOR_SCENARIOS
    if args.reminders:
        args.scenario = (args.scenario or []) + REMINDER_SCENARIOS + REMINDER_NATIVE_SCENARIOS
    if args.mac:
        args.scenario = (args.scenario or []) + MAC_SCENARIOS
    if args.web:
        args.scenario = (args.scenario or []) + WEB_SCENARIOS
    if args.vision:
        args.scenario = (args.scenario or []) + VISION_SCENARIOS
    if args.vision_native:
        args.scenario = (args.scenario or []) + VISION_NATIVE_SCENARIOS
    if args.native_media:
        args.scenario = (args.scenario or []) + NATIVE_MEDIA_SCENARIOS
    if args.maps:
        args.scenario = (args.scenario or []) + MAPS_SCENARIOS
    if args.calendar:
        args.scenario = (args.scenario or []) + CALENDAR_SCENARIOS
    if args.calendar_native:
        args.scenario = (args.scenario or []) + CALENDAR_NATIVE
    out = Path(args.output).resolve()
    out.mkdir(mode=0o700, parents=True, exist_ok=False)
    root = Path(__file__).resolve().parents[2] / 'lab' / 'scenarios'
    names = [n if '/' in n else f'core/{n}' for n in args.scenario or SCENARIOS]
    files = [str(root / f'{name}.yaml') for name in names]
    with (out / 'go.jsonl').open('w') as stream:
        impl = ['--impl', 'gateway=rust', '--rust-binary', str(Path(args.gateway).resolve())] if args.go_rust_gateway else []
        subprocess.run([args.go_lab, 'run', *files, '--level', 'sim', '--json', *impl],
                       stdout=stream, timeout=900, check=True)
    with (out / 'rust.jsonl').open('w') as stream:
        subprocess.run([args.families, 'run', *files, '--gateway', args.gateway,
                        '--simapple', args.simapple, '--output', str(out / 'worlds')],
                       stdout=stream, timeout=900, check=True)
    go, rust = verdicts(out / 'go.jsonl'), verdicts(out / 'rust.jsonl')
    if len(go) != len(files) or go != rust or not all(all(p or st == 'unverified' for p, st in v.values()) for v in rust.values()):
        raise SystemExit(f'verdict mismatch\ngo={go}\nrust={rust}')
    print(json.dumps({'scenarios': sorted(rust), 'identical': True}))


if __name__ == '__main__':
    main()
