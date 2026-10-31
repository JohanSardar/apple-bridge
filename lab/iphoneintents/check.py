import json
import pathlib
import re
import sys


def direct_commands(text):
    if not re.search(r"\bhandlers:", text):
        return set()
    return set(re.findall(r'"(iphone\.\w+(?:\.\w+)*)"', text))


def check(app):
    root = pathlib.Path(__file__).resolve().parents[2]
    commands = {"iphone.ping"}
    intents = {}
    for source in (root / "ios/Capabilities").glob("*/*.swift"):
        text = source.read_text()
        if source.name.endswith("Intents.swift"):
            for name, command in re.findall(r'struct (\w+): AppIntent.*?CommandIntent.run\("(iphone\.[\w.]+)"', text, re.S):
                intents[command] = name
        elif source.name.endswith("Capability.swift"):
            commands.update(direct_commands(text))
    assert commands == intents.keys(), "Direct command/intent coverage mismatch"
    metadata = json.loads((app / "Metadata.appintents/extract.actionsdata").read_text())
    actions = metadata["actions"]
    assert all(name in actions for name in intents.values()), "Compiled intent missing"
    assert len(intents) == len(set(intents.values())), "Intent identifiers must be unique"
    shortcuts = {item["actionIdentifier"] for item in metadata["autoShortcuts"]}
    provider = (root / "ios/Capabilities/Siri/SiriShortcuts.swift").read_text()
    registered = set(re.findall(r'AppShortcut\(intent: (\w+)\(', provider))
    assert shortcuts == registered, "Compiled Siri shortcut missing"
    print(f"intent_compile_check: pass commands={len(commands)} intents={len(intents)} shortcuts={len(shortcuts)}")


if __name__ == "__main__":
    check(pathlib.Path(sys.argv[1]))
