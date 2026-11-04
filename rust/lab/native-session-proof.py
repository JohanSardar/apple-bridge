import json
import os
import sys


def load(path):
    results = {}
    for line in open(path):
        line = line.strip()
        if not line.startswith("{"):
            continue
        try:
            value = json.loads(line)
        except ValueError:
            continue
        if "scenario" in value and "passed" in value:
            results[value["scenario"]] = value
    return results


def failed(result):
    return sorted(a["name"] for a in result["assertions"] if not a["passed"] and a.get("status") != "unverified")


def main():
    old, new = load(sys.argv[1]), load(sys.argv[2])
    both = sorted(set(old) & set(new))
    regressions = [s for s in both if old[s]["passed"] and not new[s]["passed"]]
    fixed = [s for s in both if not old[s]["passed"] and new[s]["passed"]]
    same_failures = [s for s in both if not old[s]["passed"] and not new[s]["passed"]]
    print("scenarios old=%d new=%d common=%d" % (len(old), len(new), len(both)))
    print("old pass=%d new pass=%d" % (sum(1 for s in both if old[s]["passed"]), sum(1 for s in both if new[s]["passed"])))
    print("regressions on the new path: %d" % len(regressions))
    for scenario in regressions:
        print("  REGRESSION", scenario, failed(new[scenario])[:5])
    print("failing on both paths: %d" % len(same_failures))
    for scenario in same_failures:
        print("  BOTH", scenario)
    print("passing only on the new path: %d" % len(fixed))
    if len(sys.argv) > 3:
        used = [name for name in os.listdir(sys.argv[3]) if os.path.exists(os.path.join(sys.argv[3], name, "native-session.log"))]
        print("worlds with native-session.log under %s: %d" % (sys.argv[3], len(used)))


main()
