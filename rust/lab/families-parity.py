import argparse
import json
from pathlib import Path
import re
import subprocess

HARNESS_ONLY = {'credential_backend_override_restored'}
GO_TARGET_SCENARIOS = {'RELAY-LEGACY-COMPAT'}
BLOB_SCENARIOS = ['CORE-BLOB-CAP', 'CORE-BLOB-CORRUPT', 'CORE-BLOB-DUPLICATE-REFERENCE',
                  'CORE-BLOB-GATEWAY-RETRY', 'CORE-BLOB-GC', 'CORE-BLOB-INTERRUPT',
                  'CORE-BLOB-NO-DIGEST-AUTH', 'CORE-BLOB-OFFLOADED', 'CORE-BLOB-REFERENCE-INDEX',
                  'CORE-BLOB-SLOW-READ', 'CORE-BLOB-STAGE-DURING-TRANSFER',
                  'CORE-BLOB-STAGE-SAME-DIGEST']
JOURNEY_SCENARIOS = ['CORE-CMD-IDEMPOTENT', 'CORE-CMD-ERA', 'CORE-CMD-LOST-RESPONSE', 'CORE-CMD-CONFLICT',
                     'CORE-CMD-READ-YOUR-WRITE', 'CORE-CMD-CRASH-AMBIGUOUS', 'CORE-CMD-EXPIRY',
                     'CORE-CMD-CLOCK-JUMP', 'CORE-CMD-OFFLINE', *BLOB_SCENARIOS]
JOURNEY_RUST_ONLY = {'world_started'}
RECOVERY_SCENARIOS = ['CORE-RESTORE-EPOCH', 'CORE-RETENTION-RACE', 'CORE-SCOPE-CHANGE', 'CORE-SCOPE-LEAK',
                    'CORE-SLOW-CONSUMER', 'CORE-SNAPSHOT-INTERRUPT', 'CORE-SNAPSHOT-UNDER-LOAD',
                    'CORE-UPGRADE', 'CORE-RESTORE-OLD-SCHEMA', 'CORE-SOURCE-GRANTS']
CORE_SCENARIOS = ['CORE-RESUME', 'CORE-STALE-FENCE', 'LAB-MALFORMED-SCENARIO', 'LAB-SSE',
                  'LAB-TRUTH-MULTIPLICITY', *RECOVERY_SCENARIOS]
IMPORT_SCENARIOS = ['messages/MSG-HISTORY-IMPORT', 'contacts/CON-IMPORT', 'notes/NOTE-IMPORT', 'usage/USE-IMPORT']
SHORTCUT_SCENARIOS = ['shortcuts/FILES-TRANSFORM', 'shortcuts/SC-ACTIONS', 'shortcuts/SC-CAPTURE', 'shortcuts/SC-RUN-REAL',
                      'shortcuts/CLK-SIM', 'shortcuts/CLK-ALARM-REAL', 'shortcuts/CLK-TIMER-REAL',
                      'shortcuts/SC-RUN', 'shortcuts/FOC-SIM', 'shortcuts/FOC-PHONE-SYNC']
CONTACTS_SCENARIOS = ['contacts/CON-CRUD', 'contacts/CON-CONFLICT', 'contacts/CON-DROP-EVERYTHING',
                      'contacts/CON-EXTERNAL', 'contacts/CON-GROUPS', 'contacts/CON-HANDLE-LINK',
                      'contacts/CON-IMAGE', 'contacts/CON-PERMISSION-LOSS',
                      'contacts/CON-READONLY-CONTAINER', 'contacts/CON-RESOLVE-RESTART',
                      'contacts/CON-SEARCH', 'contacts/CON-TOKEN-CRASH',
                      'contacts/CON-RECIPIENT-LOOKUP']
NOTES_SCENARIOS = ['notes/NOTE-CRUD', 'notes/NOTE-RICH-REJECT', 'notes/NOTE-EXTERNAL', 'notes/NOTE-EXTRAS',
                   'notes/NOTE-LIST', 'notes/NOTE-EDIT-PARITY']
PIM_NATIVE_SCENARIOS = ['notes/NOTE-DECODE', 'notes/NOTE-DECODE-FULL', 'mail/MAIL-CAPTURE', 'mail/MAIL-BODY', 'mail/MAIL-COMMANDS',
                        'mail/MAIL-IDLE', 'mail/MAIL-LIVE', 'mail/MAIL-NO-FALSE-DELETE']
MEDIA_SCENARIOS = ['web/WEB-MEDIA-VARIANTS', 'messages/MSG-MEDIA-CLEANUP']
WEB_RELAY_SCENARIOS = ['web/WEB-RELAY-AGENTS', 'web/WEB-RELAY-AUTH', 'web/WEB-RELAY-PAIRING']
WEB_RM_ITEMS_SCENARIOS = ['web/WEB-RM-REMINDERS-ITEMS']
WEB_RM_SCENARIOS = ['web/WEB-RM-MEDIA-COLLECTION', 'web/WEB-RM-SEARCH', 'web/WEB-RM-THREAD-ORIGINATOR',
                    'web/WEB-RM-THREAD-ORIGINATOR-PERF', 'web/WEB-RM-REMINDERS-DEMO',
                    'web/WEB-RM-REMINDERS-HARDENING', 'web/WEB-RM-REMINDERS-LISTS']
NATIVE_SCENARIOS = ['packaging/Q1-SETUP', 'packaging/R1-RELEASE-SIM', 'files/FILE-IDLE', 'files/FILE-CAPTURE',
                    'safari/SAF-CAPTURE', 'safari/SAF-IDLE',
                    'core/CORE-SOURCE-SUPPORT', 'core/CORE-SHORTCUT-SIGN']
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
                   'RELAY-CT-INDEPENDENT', 'RELAY-CAA-ACCOUNT', 'RELAY-TLS-SPLICE', 'RELAY-DOWNGRADE',
                   'RELAY-LEGACY-COMPAT', 'RELAY-ACME', 'RELAY-OWN-DOMAIN']
RELAY_STREAM_SCENARIOS = ['RELAY-STREAMS']
SECURITY_SCENARIOS = ['SEC-REMOTE-ACCESS']
CONSENT_SCENARIOS = ['consent-connector']
CONNECTOR_SCENARIOS = ['AUTH-SCOPES-METADATA', 'CONN-CHATGPT', 'CONN-CLAUDE-AI', 'RMT-PRIVATE-CONNECT']
REMINDER_SCENARIOS = ['REM-CONFLICT', 'REM-EXTERNAL-RACE', 'REM-NATIVE-LIST-RACE', 'REM-OPERATION-SHAPES',
                      'REM-READONLY-LIST']
REMINDER_NATIVE_SCENARIOS = ['core/REM-CRUD', 'reminders/REM-CRUD', 'reminders/REM-CREATE-EXTERNAL',
                             'reminders/REM-LIST-MANAGE', 'reminders/REM-LIST-CREATE', 'reminders/REM-RECURRING',
                             'reminders/REM-PERMISSION-LOSS', 'reminders/REM-DST']
REMINDER_MAC_SCENARIOS = ['reminders/REM-SECTIONS', 'reminders/REM-STORE-READ', 'reminders/REM-STORE-DECODE',
                          'reminders/REM-STORE-PERF']
MAC_SCENARIOS = ['mac/MAC-ACTIONS', 'mac/MAC-COMMANDS', 'mac/MAC-CAPTURE', 'mac/MAC-IDLE',
                 'mac/MAC-EXTRAS-SIM', 'mac/IWORK-REAL']
WEB_SCENARIOS = ['web/WEB-CON-LIST', 'web/WEB-MEDIA-CAP']
NATIVE_MEDIA_SCENARIOS = ['web/NATIVE-MEDIA-EXPORT-SAFETY', 'web/NATIVE-MEDIA-POSTERS', 'web/NATIVE-MEDIA-YIELD', 'web/WEB-REM-PARITY']
MESSAGES_B_SCENARIOS = [f'messages/MSG-{name}' for name in [
    'REACT', 'READSTATE-REAL', 'RECEIPTS', 'REPLY', 'RESEND', 'SELFTEST', 'SEND-FILES', 'SEND-GHOST',
    'SEND-IDENTICAL-MANUAL', 'SEND-NEW-HANDLE', 'SEND-SELF-EMAIL', 'SEND-SELF-TEXT', 'UI-INTERFERENCE', 'UNSEND']]
MESSAGES_B_NATIVE = ['messages/MSG-WATCHER-REARM', 'messages/MSG-DELETE-VISIBILITY', 'messages/MSG-READ-RICH-FIELDS']
MAPS_SCENARIOS = ['maps/MAP-SIM', 'maps/LOC-SIM', 'files/FILE-COMMANDS', 'files/FILE-CRUD-REAL',
                  'files/FILE-OFFLOAD-REAL']
CALENDAR_SCENARIOS = ['calendar/' + n for n in [
    'CAL-ALLDAY', 'CAL-ALLDAY-FUTURE', 'CAL-CREATE-EXTERNAL', 'CAL-CRUD', 'CAL-DST', 'CAL-FUTURE-RULE-MISMATCH',
    'CAL-ID-CHURN', 'CAL-LARGE-CAPTURE', 'CAL-MOVED-OCCURRENCE', 'CAL-MOVED-OUTSIDE-WINDOW',
    'CAL-PERIODIC-VERIFICATION', 'CAL-PERMISSION-LOSS', 'CAL-READONLY', 'CAL-RECUR-ALL-REJECT',
    'CAL-RECUR-FUTURE', 'CAL-RECUR-THIS', 'CAL-SPLIT-ATOMIC-ALLDAY', 'CAL-SPLIT-ATOMIC-TIMED',
    'CAL-SPLIT-FRAME-CAP', 'CAL-STAMP-FIELDS', 'CAL-WINDOW-SLIDE']] + ['web/WEB-CAL-WRITE']
CALENDAR_NATIVE = ['calendar/CAL-STORE-READ', 'calendar/CAL-FUTURE-COUNT-MATH', 'calendar/CAL-HINT-COVERAGE',
                   'calendar/CAL-INVALIDATED-EVENT']
VISION_SCENARIOS = ['vision/AUD-SIM', 'vision/PDF-ANNOTATE', 'vision/PDF-TEXT', 'vision/VIS-OCR']
VISION_NATIVE_SCENARIOS = ['vision/CAP-SIM']
WEB_AGENT_SCENARIOS = ['web/WEB-AGENT-CREDENTIALS', 'web/WEB-AGENT-LEVELS', 'web/WEB-AGENT-ACTIVITY',
                      'web/WEB-AGENT-OAUTH', 'web/WEB-AGENT-OAUTH-CIMD']
WEB_RM_LATENCY_SCENARIOS = ['web/WEB-RM-CHATS', 'web/WEB-RM-THREAD']
SCENARIOS = ['CORE-QUERY-ROUNDTRIP', 'CORE-QUERY-BLOB', 'CORE-QUERY-SCOPE', 'CORE-QUERY-DEADLINE',
             *JOURNEY_SCENARIOS, *CORE_SCENARIOS, *CONTACTS_SCENARIOS, *NOTES_SCENARIOS]
FRONTDOOR_SCENARIOS = ['CORE-REST-MCP-PARITY', 'CORE-LOCALHOST-DEFENSE']
SCENARIOS += FRONTDOOR_SCENARIOS
ROBUST_SCENARIOS = ['CORE-KILL-DURING-APPLY', 'CORE-KILL-GATEWAY-CAPTURE', 'CORE-HELPER-RESTART', 'CORE-PAIR']
CORE_BULK_SCENARIOS = ['CORE-ACK-DURABLE', 'CORE-CAPTURE-ALIASES', 'CORE-CREDENTIAL-STORE', 'CORE-BOOT',
                       'CORE-DISCONNECT-DETECT', 'CORE-DOCTOR', 'CORE-CLOCK-DELAY']
SCENARIOS += CORE_BULK_SCENARIOS
CORE_BULK2_SCENARIOS = ['CORE-GAP-REJECT', 'CORE-FEED-PIPELINED-ACKS', 'CORE-FORGED-DELEGATION', 'CORE-GATEWAY-DIALS',
                        'CORE-CLOCK-DELAY-GATEWAY-DIALS', 'CORE-BENCH']
SCENARIOS += CORE_BULK2_SCENARIOS
DARWIN_CORE_SCENARIOS = ['CORE-DISK-FULL', 'CORE-BLOB-NATIVE-STAGE']
SCENARIOS += ROBUST_SCENARIOS
REAL_ONLY_COMPILED = ['CORE-PERMISSION-STATUS', 'CORE-QUERY-HELPER', 'APP-ALARM-REAL', 'APP-APPROVE-REAL', 'APP-CAPTURE-REAL', 'APP-CMD-REAL',
    'APP-DEVICE-REAL', 'APP-HEALTH-REAL', 'APP-HEALTH-WRITE-REAL', 'APP-INTENTS-REAL', 'APP-LIVE-REAL',
    'APP-LOC-REAL', 'APP-MOTION-REAL', 'APP-NOTIFY-REAL', 'APP-PAIR-REAL', 'APP-SCREENTIME-REAL',
    'APP-SHELL-REAL', 'AUD-TRANSCRIBE', 'BT-REAL', 'CALL-READ-REAL', 'CALLS-REPLICA', 'CONN-CLAUDE-CODE',
    'CONN-RELAY-CLAUDE', 'FILE-REPLICA', 'RELAY-REAL',
    'WEB-RELAY-COMMAND', 'WEB-RELAY-CONNECT', 'WEB-RELAY-NATIVE-AGENTS', 'WEB-RELAY-PUBLIC-AGENTS',
    'WEB-RELAY-REPLICA', 'WEB-RELAY-REPLICA-BLOBS', 'WEB-REMOTE-FUNNEL', 'FILE-SEARCH-REAL', 'HELPER-LIVE-BACKFILL',
    'HELPER-RELEASE-COMMANDS', 'HELPER-RESTART-REAL', 'HELPER-SESSION-SOAK', 'HELPER-SIGNED-APP-CACHE',
    'HELPER-TCC-READY', 'LOC-CURRENT-REAL', 'LOC-GEOCODE-REAL', 'LOC-IDLE', 'MAC-APPEARANCE-REAL',
    'MAC-APPS-REAL', 'MAC-AUDIO-REAL', 'MAC-CLIPBOARD-REAL', 'MAC-KEEPAWAKE-REAL', 'MAC-NOTIFY-REAL',
    'MAC-POWER-REAL', 'MAC-REPLICA', 'MAIL-REPLICA', 'MAP-DIRECTIONS-REAL', 'MAP-IDLE', 'MAP-SEARCH-REAL',
    'MAP-SNAPSHOT-REAL', 'PKG-BACKGROUND', 'PKG-INSTALL', 'R1-DMG-REAL', 'REM-LIST-LOST-ACK',
    'RMT-STATUS-OFF', 'SAF-BOOKMARKS-REAL', 'SAF-HISTORY-REAL', 'SHAZAM-REAL', 'SPEECH-SAY', 'TM-STATUS',
    'VIS-IDLE', 'WEB-REM-WRITE']
REAL_ONLY_STUB = ['APP-BUILD-REAL', 'CON-ME', 'CON-NOTE', 'CON-RELAY', 'FOC-IDLE', 'FOC-SET-REAL',
    'FOC-STATUS-REAL', 'MAIL-AGENT', 'MAIL-BODY-REAL', 'MAIL-DRAFT-REAL', 'MAIL-ORGANIZE-REAL',
    'MAIL-READ-REAL', 'MAIL-SEND-SELF-REAL', 'SC-IDLE']
SCENARIOS += ['CORE-SIM-PROGRESS-RECOVERY', 'CORE-SNAPSHOT-POOL']
MOVED_TO_RUST_TESTS = {
    'core/CORE-SEARCH-UPDATE-PERF': 'drives the Go state store in-process; the Rust runner has no store access. Ported to rust/crates/ab-state/tests/search_update_perf.rs (make rust-search-perf). Speed numbers are recorded, not enforced. The Go assertions search_unavailable_during_rebuild, search_rest_preparing_is_retryable_error_not_empty, search_mcp_preparing_is_retryable_error_not_empty and restart_resumes_pending_rebuild have no Rust equivalent because ab-state rebuilds the search index inside open() instead of in a background migration.',
}
CAPTURE_SCENARIOS = ['CORE-REPLICA-CAPTURE-ATOMIC', 'CORE-LIVE-LATENCY', 'CORE-MESSAGES-SHAPE',
                     'CORE-REPLICA-CAPTURE-STAGING', 'CORE-REPLICA-CAPTURE-LEGACY', 'CORE-REPLICA-CAPTURE-OBSERVERS']
BENCH_SCENARIOS = ['CORE-READMODEL-UPDATE-PERF']
REPLICA_SCENARIOS = ['CORE-QUERY-REPLICA', 'CORE-CRED-LIFECYCLE']


def scenario_key(root, name):
    name = name[2:] if name.startswith('./') else name
    if '/' in name:
        return name
    for key in (f'core/{name}', name):
        if (root / f'{key}.yaml').is_file():
            return key
    found = [path.relative_to(root).with_suffix('').as_posix() for path in sorted(root.rglob('*.yaml'))
             if re.search(rf'^id:\s*{re.escape(name)}\s*$', path.read_text(), re.M)]
    if len(found) != 1:
        raise SystemExit(f'{name}: expected one scenario by file name or id under lab/scenarios, found {len(found)}')
    return found[0]


def ignored(scenario, name):
    if name in HARNESS_ONLY or name == 'world_started':
        return True
    if scenario in GO_TARGET_SCENARIOS and name.startswith('go_'):
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


def check_real_only(families):
    if not families:
        raise SystemExit('--families is required')
    root = Path(__file__).resolve().parents[2] / 'lab' / 'scenarios'
    listed = {**{n: 'compiled' for n in REAL_ONLY_COMPILED}, **{n: 'stub' for n in REAL_ONLY_STUB}}
    if len(listed) != len(REAL_ONLY_COMPILED) + len(REAL_ONLY_STUB):
        raise SystemExit('a scenario is listed twice across REAL_ONLY_COMPILED and REAL_ONLY_STUB')
    paths = []
    for name in listed:
        found = sorted(root.glob(f'*/{name}.yaml'))
        if len(found) != 1:
            raise SystemExit(f'{name}: expected one YAML under lab/scenarios, found {len(found)}')
        paths.append(str(found[0]))
    result = subprocess.run([families, 'check', *paths], capture_output=True, text=True, timeout=120)
    rows = {}
    for line in result.stdout.splitlines():
        if line.startswith('{'):
            row = json.loads(line)
            rows[row['scenario']] = row
    problems = []
    for name, want in listed.items():
        row = rows.get(name)
        if row is None:
            problems.append(f'{name}: not checked')
        elif not row['ok']:
            problems.append(f"{name}: {row['kind']}")
        elif row['kind'] != want:
            problems.append(f"{name}: listed as {want} but the Rust runner reports {row['kind']}")
    if problems:
        raise SystemExit('real-only check failed\n' + '\n'.join(problems))
    print(json.dumps({'compiled': len(REAL_ONLY_COMPILED), 'stub': len(REAL_ONLY_STUB), 'checked': True}))


def main():
    parser = argparse.ArgumentParser()
    for name in ['families', 'go-lab', 'gateway', 'simapple', 'output']:
        parser.add_argument('--' + name)
    parser.add_argument('--check-real-only', dest='check_real_only', action='store_true')
    parser.add_argument('--rust-only', dest='rust_only', action='store_true')
    parser.add_argument('--scenario', action='append')
    parser.add_argument('--timeout', type=int, default=900)
    parser.add_argument('--import', dest='imports', action='store_true')
    parser.add_argument('--messages-a', dest='messages_a', action='store_true')
    parser.add_argument('--messages-a-commands', dest='messages_a_commands', action='store_true')
    parser.add_argument('--media', dest='media', action='store_true')
    parser.add_argument('--messages-b', dest='messages_b', action='store_true')
    parser.add_argument('--messages-b-native', dest='messages_b_native', action='store_true')
    parser.add_argument('--web-rm', dest='web_rm', action='store_true')
    parser.add_argument('--web-rm-items', dest='web_rm_items', action='store_true')
    parser.add_argument('--web-relay', dest='web_relay', action='store_true')
    parser.add_argument('--web-rm-latency', dest='web_rm_latency', action='store_true')
    parser.add_argument('--native', dest='native', action='store_true')
    parser.add_argument('--iphone', dest='iphone', action='store_true')
    parser.add_argument('--maps', dest='maps', action='store_true')
    parser.add_argument('--shortcuts', dest='shortcuts', action='store_true')
    parser.add_argument('--relay', dest='relay', action='store_true')
    parser.add_argument('--relay-streams', dest='relay_streams', action='store_true')
    parser.add_argument('--go-rust-gateway', dest='go_rust_gateway', action='store_true')
    parser.add_argument('--connectors', dest='connectors', action='store_true')
    parser.add_argument('--consent', dest='consent', action='store_true')
    parser.add_argument('--security', dest='security', action='store_true')
    parser.add_argument('--reminders', dest='reminders', action='store_true')
    parser.add_argument('--mac', dest='mac', action='store_true')
    parser.add_argument('--reminders-mac', dest='reminders_mac', action='store_true')
    parser.add_argument('--usage', action='store_true')
    parser.add_argument('--darwin-usage', dest='darwin_usage', action='store_true')
    parser.add_argument('--calls', action='store_true')
    parser.add_argument('--helper', action='store_true')
    parser.add_argument('--web', dest='web', action='store_true')
    parser.add_argument('--native-media', dest='native_media', action='store_true')
    parser.add_argument('--bench', action='store_true')
    parser.add_argument('--replica', action='store_true')
    parser.add_argument('--core-capture', dest='core_capture', action='store_true')
    parser.add_argument('--calendar', dest='calendar', action='store_true')
    parser.add_argument('--calendar-native', dest='calendar_native', action='store_true')
    parser.add_argument('--vision', action='store_true')
    parser.add_argument('--vision-native', dest='vision_native', action='store_true')
    parser.add_argument('--web-agents', dest='web_agents', action='store_true')
    parser.add_argument('--darwin-core', dest='darwin_core', action='store_true')
    args = parser.parse_args()
    if args.check_real_only:
        check_real_only(args.families)
        return
    for name in ['families', 'go_lab', 'gateway', 'simapple', 'output']:
        if args.rust_only and name == 'go_lab':
            continue
        if not getattr(args, name):
            parser.error('--' + name.replace('_', '-') + ' is required')
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
    if args.web_relay:
        args.scenario = (args.scenario or []) + WEB_RELAY_SCENARIOS
    if args.web_rm_items:
        args.scenario = (args.scenario or []) + WEB_RM_ITEMS_SCENARIOS
    if args.web_rm:
        args.scenario = (args.scenario or []) + WEB_RM_SCENARIOS
    if args.web_rm_latency:
        args.scenario = (args.scenario or []) + WEB_RM_LATENCY_SCENARIOS
    if args.native:
        args.scenario = (args.scenario or []) + NATIVE_SCENARIOS
    if args.iphone:
        args.scenario = (args.scenario or []) + IPHONE_SCENARIOS
    if args.relay:
        args.scenario = (args.scenario or []) + RELAY_SCENARIOS
    if args.relay_streams:
        args.scenario = (args.scenario or []) + RELAY_STREAM_SCENARIOS
    if args.security:
        args.scenario = (args.scenario or []) + SECURITY_SCENARIOS
    if args.consent:
        args.scenario = (args.scenario or []) + CONSENT_SCENARIOS
    if args.connectors:
        args.scenario = (args.scenario or []) + CONNECTOR_SCENARIOS
    if args.reminders:
        args.scenario = (args.scenario or []) + REMINDER_SCENARIOS + REMINDER_NATIVE_SCENARIOS
    if args.reminders_mac:
        args.scenario = (args.scenario or []) + REMINDER_MAC_SCENARIOS
    if args.mac:
        args.scenario = (args.scenario or []) + MAC_SCENARIOS
    if args.web:
        args.scenario = (args.scenario or []) + WEB_SCENARIOS
    if args.vision:
        args.scenario = (args.scenario or []) + VISION_SCENARIOS
    if args.vision_native:
        args.scenario = (args.scenario or []) + VISION_NATIVE_SCENARIOS
    if args.core_capture:
        args.scenario = (args.scenario or []) + CAPTURE_SCENARIOS
    if args.replica:
        args.scenario = (args.scenario or []) + REPLICA_SCENARIOS
    if args.bench:
        args.scenario = (args.scenario or []) + BENCH_SCENARIOS
    if args.native_media:
        args.scenario = (args.scenario or []) + NATIVE_MEDIA_SCENARIOS
    if args.maps:
        args.scenario = (args.scenario or []) + MAPS_SCENARIOS
    if args.calendar:
        args.scenario = (args.scenario or []) + CALENDAR_SCENARIOS
    if args.calendar_native:
        args.scenario = (args.scenario or []) + CALENDAR_NATIVE
    if args.web_agents:
        args.scenario = (args.scenario or []) + WEB_AGENT_SCENARIOS
    if args.darwin_core:
        args.scenario = (args.scenario or []) + DARWIN_CORE_SCENARIOS
    root = Path(__file__).resolve().parents[2] / 'lab' / 'scenarios'
    names = [scenario_key(root, n) for n in args.scenario or SCENARIOS]
    for key in names:
        if key in MOVED_TO_RUST_TESTS:
            raise SystemExit(f'{key} is not run by families-parity: {MOVED_TO_RUST_TESTS[key]}')
    out = Path(args.output).resolve()
    out.mkdir(mode=0o700, parents=True, exist_ok=False)
    files = [str(root / f'{name}.yaml') for name in names]
    if not args.rust_only:
        with (out / 'go.jsonl').open('w') as stream:
            impl = ['--impl', 'gateway=rust', '--rust-binary', str(Path(args.gateway).resolve())] if args.go_rust_gateway else []
            level = ['--level', 'replica', '--local-replicas'] if args.replica else ['--level', 'sim']
            subprocess.run([args.go_lab, 'run', *files, *level, '--json', *impl],
                           stdout=stream, timeout=args.timeout, check=True)
    with (out / 'rust.jsonl').open('w') as stream:
        subprocess.run([args.families, 'run', *files, '--gateway', args.gateway,
                        '--simapple', args.simapple, '--output', str(out / 'worlds')],
                       stdout=stream, timeout=args.timeout, check=True)
    rust = verdicts(out / 'rust.jsonl')
    if args.rust_only:
        failing = {name: sorted(a for a, (passed, status) in checks.items() if not passed and status != 'unverified') for name, checks in rust.items()}
        failing = {name: bad for name, bad in failing.items() if bad}
        if len(rust) != len(files) or failing:
            raise SystemExit(f'rust verdicts failed\n{failing}')
        print(json.dumps({'scenarios': sorted(rust), 'rust_only': True}))
        return
    go = verdicts(out / 'go.jsonl')
    if len(go) != len(files) or go != rust or not all(all(p or st == 'unverified' for p, st in v.values()) for v in rust.values()):
        raise SystemExit(f'verdict mismatch\ngo={go}\nrust={rust}')
    print(json.dumps({'scenarios': sorted(rust), 'identical': True, 'moved': MOVED_TO_RUST_TESTS}))


if __name__ == '__main__':
    main()
