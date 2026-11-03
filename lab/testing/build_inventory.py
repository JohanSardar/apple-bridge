import argparse
from collections import Counter, defaultdict
import json
from pathlib import Path
import re


def generate(manifest):
    data = json.loads(Path(manifest).read_text())
    features = {f['id']: f for f in data['features']['features']}
    measurements = Path('lab/testing/measurements.json')
    timed_out = set()
    if measurements.exists():
        history = json.loads(measurements.read_text())
        timed_out = {row['id'] for run in history['runs'] for row in run['results'] if row['status'] == 'timeout'}
    sources = {str(p): p.read_text() for p in Path('lab/scenarios').glob('*.go') if not p.name.endswith('_test.go')}
    dispatcher = sources['lab/scenarios/runner.go']
    functions = {}
    for path, source in sources.items():
        matches = list(re.finditer(r'^func (\w+)\(', source, re.M))
        for i, match in enumerate(matches):
            body = source[match.start():matches[i + 1].start() if i + 1 < len(matches) else len(source)]
            functions[match[1]] = (path, body)
    dependencies = defaultdict(set)
    for package_file in Path('internal').rglob('*.go'):
        if not package_file.name.endswith('_test.go'):
            dependencies[str(package_file.parent)].update(re.findall(r'"github.com/JohanSardar/apple-bridge/(internal/[^\"]+)"', package_file.read_text()))
    swift_dependencies = {}
    for name, dependencies_text in re.findall(r'\.(?:target|executableTarget)\(name: "([^"]+)", dependencies: \[([^\]]*)\]', Path('native/Package.swift').read_text()):
        swift_dependencies[name] = set(re.findall(r'"([^"]+)"', dependencies_text))
    selection = defaultdict(list)
    entries = []
    for scenario in data['scenarios']:
        identity, feature = scenario['ID'], scenario['Feature']
        candidates = sorted(p for p in Path('lab/scenarios').rglob('*.yaml') if re.search(r'^id:\s*' + re.escape(identity) + r'\s*$', p.read_text(), re.M))
        path = next((p for p in candidates if ('levels: [sim]' in p.read_text()) == ('sim' in scenario['Levels'])), candidates[0])
        declaration = features.get(feature, {})
        modules = declaration.get('modules', [])
        roots = set()
        for match in re.finditer(r'if ([^\n]+) \{\s*return (\w+)\(', dispatcher):
            condition, root = match.groups()
            if '"' + identity + '"' in condition or '"' + feature + '"' in condition or any(identity.startswith(p) for p in re.findall(r'HasPrefix\(s.ID, "([^"]+)"', condition)):
                roots.add(root)
        if not roots:
            roots.add('runScenario')
        reached = set()
        pending = list(roots)
        while pending:
            name = pending.pop()
            if name in reached or name not in functions or name == 'runScenario':
                continue
            reached.add(name)
            body = functions[name][1]
            pending.extend(n for n in re.findall(r'\b(\w+)\(', body) if n in functions and n not in reached)
        paths = sorted({functions[n][0] for n in reached})
        imports = sorted({p for source in paths for p in re.findall(r'"github.com/JohanSardar/apple-bridge/(internal/[^\"]+)"', sources[source])})
        packages = set(imports)
        swift = set()
        for module in modules:
            if 'apple-bridge-iphone/tree/main/' in module:
                swift.add('iPhone:' + module.split('/tree/main/', 1)[1])
            elif module.startswith('native/'):
                swift.add(module.split('/')[-1])
            elif Path('internal', module).is_dir():
                packages.add('internal/' + module)
            elif module.startswith('internal/'):
                packages.add(module)
            elif Path(module).is_dir() and any(Path(module).glob('*.go')):
                packages.add(module)
        packages.update(['internal/contracts'])
        pending_packages = list(packages)
        while pending_packages:
            package = pending_packages.pop()
            for dependency in dependencies.get(package, set()) - packages:
                packages.add(dependency)
                pending_packages.append(dependency)
        native_features = {'phone.read', 'recordings.read', 'safari.capture', 'usage.capture', 'usage.action_probe', 'core.permissions', 'core.shortcuts', 'helper.siri_intents', 'packaging.app', 'packaging.release', 'packaging.setup', 'iphone.alarms', 'iphone.screentime', 'iphone.health', 'iphone.health_write', 'iphone.location', 'iphone.live', 'iphone.device', 'iphone.motion', 'clock.pack'}
        native = feature in native_features or identity in {'APP-NOTIFY-SIM', 'FOC-PHONE-SYNC', 'FOC-SIM', 'MSG-DECODE', 'MSG-DECODE-BALLOON', 'MSG-DECODE-FORMATTING', 'MSG-DELETE-VISIBILITY', 'MSG-READ-RICH-FIELDS', 'MSG-WATCHER-REARM', 'RELAY-ACME', 'RELAY-OWN-DOMAIN', 'CORE-SOURCE-SUPPORT', 'WEB-MEDIA-VARIANTS', 'CAP-SIM', 'FILES-TRANSFORM', 'MAIL-BODY', 'MAIL-CAPTURE', 'MAIL-COMMANDS', 'MAIL-IDLE', 'MAIL-LIVE', 'MAIL-NO-FALSE-DELETE', 'MSG-MEDIA-CLEANUP', 'REM-SECTIONS', 'SC-ACTIONS', 'SC-CAPTURE', 'SC-RUN', 'CAL-FUTURE-COUNT-MATH', 'CAL-HINT-COVERAGE', 'CAL-INVALIDATED-EVENT', 'CORE-DISK-FULL', 'FILE-CAPTURE', 'FILE-IDLE', 'USE-SEGB-DECODE', 'NOTE-DECODE', 'NOTE-DECODE-FULL', 'NOTE-DECODER', 'APP-SHELL-SIM', 'APP-CAPTURE-SIM', 'APP-DEVICE-SIM', 'APP-MOTION-SIM', 'APP-HEALTH-WRITE-SIM', 'REM-DST', 'NATIVE-MEDIA-YIELD', 'NATIVE-MEDIA-POSTERS', 'NATIVE-MEDIA-EXPORT-SAFETY', 'HELPER-SIGNED-APP-CACHE', 'REM-STORE-PERF', 'REM-STORE-READ', 'REM-STORE-DECODE', 'CAL-STORE-READ', 'WEB-REM-PARITY'}
        pending_swift = list(swift)
        while pending_swift:
            module = pending_swift.pop()
            for dependency in swift_dependencies.get(module, set()) - swift:
                swift.add(dependency)
                pending_swift.append(dependency)
        text = path.read_text()
        semantic_calls = sorted({call for name in reached for call in re.findall(r'contracts\.(CommandHash|Descriptor|Grant|GrantsForSource|HealthSample|IPhoneCompletionEvidence|MacCompletionEvidence|MacDescriptors|NormalizeContactHandle|OperationSchema|ValidateCommand|ValidateIPhoneCommand|BlobReferences)\(', functions[name][1])})
        native_fixture = native and identity != 'CORE-SHORTCUT-SIGN'
        whitebox = bool(set(imports) - {'internal/contracts'}) or native_fixture or bool(semantic_calls)
        recommendation, reason = 'keep', 'Distinct journey; retain its behavioral assertions.'
        if declaration.get('status') == 'dropped':
            recommendation, reason = 'remove', 'Feature is explicitly dropped in features.yaml; owner must confirm retained compatibility need.'
        elif identity == 'RELAY-STREAMS':
            recommendation, reason = 'run-at-end', 'Its measured ten-minute stream observation is an integration soak; a 600-second outer limit was too short.'
        elif identity == 'WEB-RM-THREAD':
            recommendation, reason = 'run-at-end', 'The functional journey includes a 170000-message archive and a 10 ms p95 budget; preserve it as a serial end gate.'
        elif identity in {'WEB-RM-CHATS', 'WEB-RM-REMINDERS-ITEMS', 'CAL-STORE-READ'}:
            recommendation, reason = 'run-at-end', 'The functional journey embeds a 10-15 ms p95 benchmark; preserve all assertions in a serial end gate.'
        elif identity == 'MSG-WATCHER-REARM':
            recommendation, reason = 'run-at-end', 'Native watcher fault checks use short deadlines and failed in the default run while passing in isolation; retain as a serial end gate.'
        elif identity in timed_out:
            recommendation, reason = 'run-at-end', 'Observed a hard timeout; retain for isolated final diagnosis before Rust qualification.'
        elif re.search(r'IDLE|PERF|BENCH|SOAK|LATENCY', identity):
            recommendation, reason = 'run-at-end', 'Timing or resource budget needs an isolated final gate; contention can change verdicts.'
        elif whitebox:
            recommendation, reason = 'convert', ('Native fixture check compiles or links product modules; drive the binary protocol before Rust qualification.' if native and not set(imports) - {'internal/contracts'} else 'Reachable harness file imports product implementation; drive the binary before Rust qualification.')
        elif identity.startswith('WEB-') and feature.startswith('web.'):
            recommendation, reason = 'merge', 'Candidate overlap with API journey in this feature; compare assertions before consolidation.'
        overrides = {
            'CORE-REPLICA-CAPTURE-OBSERVERS': ('convert', 'Capture-observer assertions use REST SSE, consent pages and replicated command records; convert remaining implementation imports before Rust qualification.'),
            'WEB-RM-THREAD-ORIGINATOR': ('convert', 'REST/MCP originator checks and REST SSE observers preserve filtered windows; retain binary execution for Rust qualification.'),
            'WEB-AGENT-ACTIVITY': ('convert', 'Replace product state.Open activity inspection with a supported binary API or read-only SQLite oracle.'),
            'WEB-CAL-WRITE': ('convert', 'Stale browser-level metadata blocks SIM loading; preserve Calendar write assertions in the binary journey.'),
            'WEB-REM-PARITY': ('convert', 'Stale browser-level metadata blocks SIM loading; preserve Reminders parity assertions in the binary journey.'),
        }
        if identity in overrides:
            recommendation, reason = overrides[identity]
        host_reason = 'Go toolchain/Pebble prerequisite absent on Ubuntu' if identity in {'FOC-PHONE-SYNC', 'FOC-SIM', 'RELAY-ACME', 'RELAY-OWN-DOMAIN'} else ('macOS/Swift fixture dependency' if native else 'portable SIM driver')
        entry = {'id': identity, 'path': str(path), 'feature': feature, 'levels': scenario['Levels'], 'host': 'darwin' if native else 'linux', 'host_reason': host_reason, 'packages': sorted(packages), 'swift': sorted(swift), 'roots': sorted(roots), 'code_paths': paths, 'p_class': whitebox, 'semantic_contract_calls': semantic_calls, 'recommendation': recommendation, 'reason': reason}
        previous = next((e for e in entries if e['id'] == identity), None)
        if previous:
            previous['levels'] = sorted(set(previous['levels'] + entry['levels']))
            if 'sim' in scenario['Levels']:
                previous['path'] = entry['path']
        else:
            entries.append(entry)
        for key in {identity, feature, *packages, *swift}:
            if identity not in selection[key]:
                selection[key].append(identity)
    crates = {'ab-domain': ['contracts'], 'ab-wire': ['contracts', 'applepeer', 'replication', 'iphone'], 'ab-catalog': ['operations', 'frontdoor'], 'ab-state': ['state'], 'ab-trust': ['trust'], 'ab-blobs': ['blobs'], 'ab-operations': ['operations'], 'ab-applepeer': ['applepeer'], 'ab-replication': ['replication'], 'ab-frontdoor': ['frontdoor'], 'ab-remote': ['remote', 'relay'], 'ab-iphone': ['iphone'], 'ab-runtime': ['runtime'], 'ab-native': [], 'ab-session': [], 'ab-sys': []}
    for crate, packages in crates.items():
        selection[crate] = sorted({e['id'] for e in entries if any('internal/' + p in e['packages'] for p in packages) or (crate in {'ab-native', 'ab-session', 'ab-sys'} and e['swift'])})
    for shared in ['internal/runtime', 'cmd/bridge', 'cmd/bridge-lab', 'lab/world', 'lab/scenarios', 'lab/simapple', 'ab-runtime', 'lab-world', 'lab-scenarios', 'lab-simapple', 'lab-wire', 'lab-truth-client']:
        selection[shared] = sorted(e['id'] for e in entries)
    result = {'mapping': 'Conservative static candidates: features.yaml modules plus imports in same-package helper closure; imports do not prove executed product calls. Unknown dynamic paths require full suite.', 'scenarios': entries, 'selection': dict(sorted(selection.items()))}
    Path('lab/test-selection.json').write_text(json.dumps(result, separators=(',', ':')) + '\n')
    print(dict(Counter(e['recommendation'] for e in entries)))
    print(dict(Counter(e['host'] for e in entries if 'sim' in e['levels'])))


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('manifest')
    generate(parser.parse_args().manifest)
