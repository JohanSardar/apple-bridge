#!/usr/bin/env python3
"""Breaks each dangerous command rule on purpose and checks that a test goes red. Run from the repository root.

usage: mutation-check.py [name-filter]

Prints CAUGHT or MISSED per rule and restores the file. A rule with two layers is mutated at both, since either one
alone still refuses. Exits non-zero if a mutant survives.
"""
import subprocess, sys

SESSION = "rust/crates/ab-session/src/session/"
FILES = SESSION + "files.rs"
MAIL = SESSION + "mail.rs"
FOCUS = SESSION + "focus.rs"
SHORTCUTS = SESSION + "shortcuts.rs"
MAC = SESSION + "mac.rs"
MEDIA = SESSION + "media.rs"
LOOKUP = "rust/crates/ab-session/src/calendar_lookup.rs"
COMMANDS = "rust/crates/ab-commands/src/sources.rs"
WIRE = "rust/crates/ab-wire/src/native.rs"
session = ("ab-session", "--lib")
RULES = [
    ("files: a symbolic link or .. cannot leave the root", session + ("session::files",), [
        (FILES, 'Ok(metadata) if metadata.file_type().is_symlink() => return Err(Denied::Invalid),', ''),
        (FILES, 'if matches!(part, "" | "." | "..") {', 'if false {'),
        (FILES, 'if resolved != url || !resolved.starts_with(&root) || resolved == root {', 'if false {'),
    ]),
    ("files: the root must be a configured root", session + ("session::files",), [(FILES, '.find(|candidate| **candidate == clean)', '.find(|_| true)')]),
    ("files: a name stays one path component", session + ("session::files",), [(FILES, '!matches!(name, "" | "." | "..") && !name.contains(\'/\') && !name.contains(\'\\0\')', 'true')]),
    ("files: a destination that is a link refuses", session + ("session::files",), [(FILES, 'if fs::symlink_metadata(&url).is_ok_and(|metadata| metadata.file_type().is_symlink()) {', 'if false {')]),
    ("files: content must match its digest", session + ("session::files",), [(FILES, 'bytes.len() > MAX_CONTENT || digest_of(&bytes) != text(args, "digest")', 'false')]),
    ("files: create never overwrites", session + ("session::files",), [(FILES, '.create_new(true)\n        .custom_flags(ab_sys::O_NOFOLLOW)\n        .open(url)', '.create(true)\n        .truncate(true)\n        .open(url)')]),
    ("files: move never replaces", session + ("session::files",), [(FILES, 'ab_sys::rename_exclusive(&url, &goal)', 'fs::rename(&url, &goal)')]),
    ("files: search returns only paths in a root", session + ("session::files",), [(FILES, '&& roots\n                    .iter()\n                    .any(|root| path.starts_with(root) && path != root)', '')]),
    ("files: a query cannot leave its predicate", session + ("session::files",), [(FILES, 'query.replace(\'\\\\\', "\\\\\\\\").replace(\'"\', "\\\\\\"")', 'query.to_owned()')]),
    ("files: a read stays in the root", session + ("session::files",), [(FILES, 'if resolved == root || !resolved.starts_with(&root) {', 'if false {')]),
    ("files: a trash path must be in a trash folder", session + ("session::files",), [(FILES, 'if !inside || fs::symlink_metadata(trash).is_err() || fs::symlink_metadata(&url).is_ok()', 'if false')]),
    ("mail: a recipient is one bare address", ("ab-wire", "--lib", "mail_recipient"), [(WIRE, '&& !value\n            .chars()\n            .any(|c| c.is_whitespace() || c.is_control() || ",;:<>()[]\\"\\\\".contains(c))', '')]),
    ("mail: the session checks every recipient", session + ("session::mail",), [(MAIL, '.all(|address| mail_recipient(address))', '.all(|_| true)')]),
    ("mail: the gateway checks every recipient", ("ab-commands", "--lib", "mail_tests"), [(COMMANDS, '.is_some_and(ab_wire::native::mail_recipient),', '.is_some(),')]),
    ("mail: a send needs a recipient and a subject", session + ("session::mail",), [(MAIL, 'if to.is_empty()\n            || text(args, "subject").is_empty()\n', 'if false\n')]),
    ("mail: attachments, replies and forwards refuse", session + ("session::mail",), [(MAIL, 'if !text(args, "reply_to").is_empty() || !text(args, "forward_of").is_empty() {', 'if false {'), (MAIL, 'if args["attachments"]\n        .as_array()\n        .is_some_and(|files| !files.is_empty())\n    {', 'if false {')]),
    ("mail: text cannot leave its AppleScript string", session + ("session::mail",), [(MAIL, '.replace(\'"\', "\\\\\\"")', '')]),
    ("mail: a draft stays open until saved", session + ("session::mail",), [(MAIL, 'if saved then close m saving yes', 'close m saving yes')]),
    ("focus: only Do Not Disturb", session + ("session::focus",), [(FOCUS, '|| text(args, "mode") != "Do Not Disturb"\n', '')]),
    ("focus: an end time needs on and the future", session + ("session::focus",), [(FOCUS, 'if !until.is_empty() && (args["on"] != true || !future(until)) {', 'if false {')]),
    ("focus: nothing runs without the grant", session + ("session::focus",), [(FOCUS, 'text(current, "authorization") == "authorized" && !current["focused"].is_null()', 'true')]),
    ("shortcuts: a name cannot be a flag", session + ("session::shortcuts",), [(SHORTCUTS, "|| name.starts_with('-')\n", '')]),
    ("shortcuts: a blank name refuses", session + ("session::shortcuts",), [(SHORTCUTS, 'if name.trim().is_empty()\n', 'if false\n')]),
    ("shortcuts: timeout and input stay in bounds", session + ("session::shortcuts",), [(SHORTCUTS, '|| !(0.0..=MAX_TIMEOUT_MS).contains(&timeout)\n        || text(args, "input").len() > MAX_INPUT\n', '')]),
    ("mac: power actions refuse", session + ("session::mac",), [(MAC, 'if !CAPABILITIES.contains(&operation) {', 'if false {'), (MAC, '_ => return Err("unsupported"),\n    };\n    if ok', '_ => true,\n    };\n    if ok')]),
    ("mac: a notification link is http or https", session + ("session::mac",), [(MAC, 'matches!(scheme, "http" | "https")', 'true')]),
    ("sound: only named system sounds", session + ("session::media",), [(MEDIA, 'SOUNDS.contains(&text(args, "name")) && text(args, "text").is_empty()', 'text(args, "text").is_empty()')]),
    ("capture: an artifact must be in our directory", session + ("session::media",), [(MEDIA, 'if parent != directory.canonicalize().map_err(|_| corrupt())? {', 'if false {')]),
    ("calendar: another calendar is never the event", session + ("calendar_lookup",), [(LOOKUP, 'if text(candidate, "calendar") != text(identity, "calendar")\n        || text(candidate, "account") != text(identity, "account")\n    {', 'if false {')]),
    ("calendar: an occurrence matches its own date", session + ("calendar_lookup",), [(LOOKUP, 'return Ok(original == civil);', 'return Ok(true);')]),
    ("calendar: an unreadable date is never missing", session + ("calendar_lookup",), [(LOOKUP, 'let Some(moment) = date_of(captured, data["all_day"] == true, &local_zone()) else {\n        return Ok(PRESENT);\n    };', 'let Some(moment) = date_of(captured, data["all_day"] == true, &local_zone()) else {\n        return Ok(MISSING);\n    };')]),
    ("calendar: known absent needs positive proof", session + ("calendar_lookup",), [(LOOKUP, 'same_account() && missing && !captured.iter().any(|row| text(row, "key") == native)', 'true')]),
]


def main():
    wanted = sys.argv[1] if len(sys.argv) > 1 else ""
    missed = 0
    for name, test, edits in RULES:
        if wanted not in name:
            continue
        originals = {}
        applied = True
        for path, old, new in edits:
            text = open(path).read()
            originals.setdefault(path, text)
            if old not in text:
                applied = False
                print("NOT APPLIED", name, repr(old[:60]))
                break
            open(path, "w").write(text.replace(old, new, 1))
        result = None
        if applied:
            package, *rest = test
            result = subprocess.run(["cargo", "test", "--manifest-path", "rust/Cargo.toml", "-p", package, *rest], capture_output=True, text=True)
        for path, text in originals.items():
            open(path, "w").write(text)
        if not applied:
            missed += 1
            continue
        compiled = "could not compile" not in result.stderr
        caught = result.returncode != 0 and compiled
        print(("CAUGHT " if caught else ("NO BUILD " if not compiled else "MISSED ")) + name, flush=True)
        if not caught:
            missed += 1
            if not compiled:
                print("\n".join(line for line in result.stderr.splitlines() if line.startswith("error"))[:400])
    print("survived or broken:", missed)
    sys.exit(1 if missed else 0)


main()
