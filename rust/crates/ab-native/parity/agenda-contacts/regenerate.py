import hashlib
import json
import pathlib
import subprocess
import tempfile

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parents[4]
CALENDAR = ROOT / "native/Sources/CalendarSource/CalendarSource.swift"
CONTACTS = ROOT / "native/Sources/ContactsSource/ContactsSource.swift"
SUPPORT = ROOT / "native/Sources/SourceSupport/Source.swift"
TIME = ROOT / "native/Sources/CalendarSource/CalendarTime.swift"
SCOPE = ROOT / "native/Sources/CalendarSource/CalendarScanScope.swift"
STAMP = "2026-11-03T00:00:00.000Z"


def between(text, start, end):
    return text[text.index(start):text.index(end, text.index(start))]


def row(collection, key, data, alias=None):
    return {"kind": "upsert", "collection": collection, "native_key": key,
            "data": data, "scoped_alias": alias if alias is not None else "",
            "observed_at": STAMP}


def event(key, title, alias=None, start="2026-11-04T10:00:00.000Z"):
    return row("calendar.event", key, {"title": title, "start": start,
               "end": "2026-11-04T11:00:00.000Z", "all_day": False,
               "time_zone": "UTC"}, alias)


def fixtures():
    result = []
    contact_scope = ["contacts.contact", "contacts.group", "contacts.container"]
    calendar_scope = ["calendar.calendar", "calendar.event"]
    container = row("contacts.container", "c", {"name": "Synthetic", "type": 1})
    contact = row("contacts.contact", "a", {"given_name": "Fixture", "phones": [], "is_me": True}, "contacts/contacts.contact/a")
    group = row("contacts.group", "g", {"name": "Fixture Group", "members": ["a"]}, "contacts/contacts.group/g")
    new_contact = row("contacts.contact", "b", {"given_name": "Second", "phones": []}, "contacts/contacts.contact/b")
    calendar = row("calendar.calendar", "c", {"title": "Synthetic", "writable": True})
    first = event("e", "First", "source/c/external")
    second = event("f", "Second", "source/c/second")

    def add(source, name, records, revalidated=None, force=False, generation="g1", drop=False, token_nil=False, lookup=None):
        index = len(result)
        result.append({"source": source, "name": name,
                       "snapshot": {"generation": generation,
                                    "scope": contact_scope if source == "contacts" else calendar_scope,
                                    "complete": True, "records": records,
                                    "revalidated": revalidated or {}, "force": force},
                       "context": {"observed_at": STAMP, "checkpoint": f"checkpoint-{index}", "scan_id": f"scan-{index}"},
                       "drop": drop, "token_nil": token_nil, "lookup": lookup or {}})

    add("contacts", "initial", [container, contact, group], force=True, token_nil=True)
    add("contacts", "unchanged-numeric-equivalence", [container, {**contact, "data": {**contact["data"], "score": 1}}, group])
    scored = {**contact, "data": {**contact["data"], "score": 1.0}}
    add("contacts", "integer-versus-double", [container, scored, group])
    changed = {**scored, "data": {**scored["data"], "given_name": "Changed <&> /é"}}
    add("contacts", "changed-data", [container, changed, group])
    alias_changed = {**changed, "scoped_alias": "replacement-alias"}
    add("contacts", "alias-is-not-contact-data", [container, alias_changed, group])
    add("contacts", "revalidated-deletions", [container], {"contacts.contact/a": "absent", "contacts.group/g": "absent"})
    add("contacts", "generation-drop-without-false-deletion", [new_contact], generation="g2", drop=True)
    add("contacts", "container-complete-absence", [new_contact], generation="g2")
    add("contacts", "force-replay", [new_contact], force=True, generation="g2")
    add("contacts", "complete-empty", [], {"contacts.contact/b": "absent"}, generation="g2")

    add("calendar", "initial", [calendar, first, second], force=True)
    add("calendar", "unchanged", [calendar, first, second])
    changed_event = {**first, "data": {**first["data"], "title": "Changed <&> /é"}}
    add("calendar", "changed-event", [calendar, changed_event, second])
    alias_event = {**changed_event, "scoped_alias": "source/c/new-external"}
    add("calendar", "changed-scoped-alias", [calendar, alias_event, second])
    moved = {**alias_event, "native_key": "e-moved"}
    add("calendar", "same-alias-replaces-native-identity", [calendar, moved, second])
    outside = {**second["data"], "start": "2035-01-01T00:00:00.000Z", "end": "2035-01-01T01:00:00.000Z"}
    add("calendar", "revalidated-window-eviction", [calendar, moved], {"calendar.event/f": "evict"}, lookup={"f": outside})
    add("calendar", "revalidated-absent", [calendar], {"calendar.event/e-moved": "absent"})
    add("calendar", "force-replay", [calendar], force=True)
    add("calendar", "complete-empty", [], {"calendar.calendar/c": "absent"})
    return result


def build_source():
    contacts = CONTACTS.read_text()
    calendar = CALENDAR.read_text()
    scalar = between(SUPPORT.read_text(), "public enum JSON:", "public enum SourceFailure:")
    contact_changes = between(contacts, "        for key in current.keys.sorted() {", '        trace.record("contacts_compared"')
    contact_absence = between(contacts, "        if !drop {\n            let groups =", "        var handles = Dictionary")
    calendar_absence = between(calendar, "        for (identity, previous) in committed.merging(offered", "        guard gate.authorized(.event) else { throw SourceFailure.permissionMissing }\n        gate.trace.record(\"calendar_revalidation\"")
    calendar_events = between(calendar, "        for (identity, observation) in current where observation", "        let captureBytes =")
    calendar_time = TIME.read_text().replace("import SourceSupport", "")
    calendar_scope = SCOPE.read_text().replace("import SourceSupport", "")
    return "\n".join([
        "import Foundation", scalar,
        "enum SourceFailure: Error { case invalid }",
        f'func timestamp() -> String {{ "{STAMP}" }}',
        "func timestamp(_ date: Date) -> String { date.ISO8601Format(.init(includingFractionalSeconds: true)) }",
        "func parseTimestamp(_ value: String) -> Date? { let f = ISO8601DateFormatter(); f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]; return f.date(from: value) ?? ISO8601DateFormatter().date(from: value) }",
        calendar_time, calendar_scope,
        "struct Group { let identifier: String }",
        "struct Store { let identifiers: [String]; func groups(matching: String?) throws -> [Group] { identifiers.map { Group(identifier: $0) } } }",
        "@main struct Main { static func main() throws {",
        "let input = try JSONDecoder().decode(JSON.self, from: Data(contentsOf: URL(fileURLWithPath: CommandLine.arguments[1])))",
        "var baselines: [String: [String: JSON]] = [:]",
        "var results: [JSON] = []",
        "for stage in input.array {",
        "let source = stage[\"source\"].string, snapshot = stage[\"snapshot\"]",
        "let committed = baselines[source] ?? [:]",
        "var current = Dictionary(uniqueKeysWithValues: snapshot[\"records\"].array.map { ($0[\"collection\"].string + \"/\" + $0[\"native_key\"].string, $0) })",
        "let force = snapshot[\"force\"].bool, drop = stage[\"drop\"].bool",
        "var observations: [JSON] = []",
        "if source == \"contacts\" {",
        "let token: Data? = stage[\"token_nil\"].bool ? nil : Data([1])",
        "let fullScan = !stage[\"incremental\"].bool",
        "let store = Store(identifiers: current.values.filter { $0[\"collection\"].string == \"contacts.group\" }.map { $0[\"native_key\"].string })",
        "func isAbsent(_ key: String) throws -> Bool { snapshot[\"revalidated\"][\"contacts.contact/\" + key].string == \"absent\" }",
        contact_changes, contact_absence,
        "baselines[source] = drop ? committed.merging(current) { _, new in new } : current",
        "} else {",
        "let offered: [String: JSON] = [:], scope: CalendarScanScope? = nil",
        "let bounds = DateInterval(start: parseTimestamp(\"2024-11-03T00:00:00.000Z\")!, end: parseTimestamp(\"2028-11-03T00:00:00.000Z\")!)",
        "func lookup(_ native: String) throws -> JSON? { let value = stage[\"lookup\"][native]; return value == .null ? nil : value }",
        "func data(_ outside: JSON) -> JSON { outside }",
        "var pending = current.values.filter { $0[\"collection\"].string == \"calendar.calendar\" && (force || $0[\"data\"] != committed[$0[\"collection\"].string + \"/\" + $0[\"native_key\"].string]?[\"data\"]) }",
        calendar_absence,
        "let forceFull = force",
        calendar_events,
        "observations = pending + (forceFull ? unchanged : [])",
        "baselines[source] = current",
        "}",
        "observations.sort { ($0[\"collection\"].string, $0[\"native_key\"].string) < ($1[\"collection\"].string, $1[\"native_key\"].string) }",
        "var fields = stage.object; fields[\"expected\"] = .array(observations)",
        "let dataEncoder = JSONEncoder(); dataEncoder.outputFormatting = [.sortedKeys]",
        "fields[\"expected_data\"] = .array(try observations.map { .string(String(decoding: try dataEncoder.encode($0[\"data\"]), as: UTF8.self)) })",
        "results.append(.object(fields))",
        "}",
        "let encoder = JSONEncoder(); encoder.outputFormatting = [.prettyPrinted, .sortedKeys]; FileHandle.standardOutput.write(try encoder.encode(JSON.array(results)))",
        "} }",
    ])


def main():
    with tempfile.TemporaryDirectory(prefix="ab-agenda-contacts-oracle-") as temporary:
        path = pathlib.Path(temporary)
        source = path / "oracle.swift"
        source.write_text(build_source())
        fixture = path / "input.json"
        stages = fixtures()
        fixture.write_text(json.dumps(stages, ensure_ascii=False))
        binary = path / "oracle"
        subprocess.run(["swiftc", "-parse-as-library", "-swift-version", "6", "-O", str(source), "-o", str(binary)], check=True)
        output = subprocess.check_output([str(binary), str(fixture)], env={"PATH": "/usr/bin:/bin", "TZ": "UTC"})
        oracle_stages = json.loads(output)
        for stage, oracle_stage in zip(stages, oracle_stages, strict=True):
            stage["expected"] = oracle_stage["expected"]
            stage["expected_data"] = oracle_stage["expected_data"]
        (HERE / "replay.json").write_text(json.dumps(stages, indent=2, ensure_ascii=False) + "\n")
        sources = [CALENDAR, CONTACTS, SUPPORT, TIME, SCOPE]
        evidence = {"source_sha256": {str(file.relative_to(ROOT)): hashlib.sha256(file.read_bytes()).hexdigest() for file in sources},
                    "oracle": "Exact extracted Swift source diff loops; synthetic JSON facts only; no framework/store access",
                    "stages": len(stages), "comparison": "cargo test -p ab-native --test agenda_contacts_shadow",
                    "data_comparison": "Swift JSONEncoder bytes through Go JSON codec versus Rust native JSON record bytes"}
        (HERE / "evidence.json").write_text(json.dumps(evidence, indent=2) + "\n")
        print(json.dumps({"stages": evidence["stages"], "replay": str(HERE / "replay.json")}))


if __name__ == "__main__":
    main()
