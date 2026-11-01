import json
import os
import sys
import urllib.request


def activate(bundle):
    base = os.environ.get("AB_WDA_URL")
    if not base:
        return
    base = base.rstrip("/")

    def request(path, value):
        query = urllib.request.Request(base + path, data=json.dumps(value).encode(), headers={"Content-Type": "application/json"})
        with urllib.request.urlopen(query, timeout=30) as response:
            result = json.load(response)
        value = result.get("value")
        if isinstance(value, dict) and value.get("error"):
            raise RuntimeError("The physical iPhone UI automation request failed.")
        return result

    session = os.environ.get("AB_WDA_SESSION")
    if not session:
        result = request("/session", {"capabilities": {"alwaysMatch": {"shouldTerminateApp": False, "waitForIdleTimeout": 1}}})
        session = result.get("sessionId") or result.get("value", {}).get("sessionId")
    if not session:
        raise RuntimeError("The physical iPhone UI automation session did not start.")
    request("/session/" + session + "/wda/apps/activate", {"bundleId": bundle})
    result = request("/session/" + session + "/wda/apps/state", {"bundleId": bundle})
    if result.get("value") != 4:
        raise RuntimeError("Apple Bridge did not become the foreground app on the iPhone.")


if __name__ == "__main__":
    activate(sys.argv[1])
