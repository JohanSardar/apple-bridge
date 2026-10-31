import json
import os
import pathlib
import subprocess
import sys
import tempfile
import unittest


class RealRunnerTest(unittest.TestCase):
    def test_nonce_bound_final_result_and_redaction(self):
        with tempfile.TemporaryDirectory(prefix="ab-test-host-") as temporary:
            root = pathlib.Path(temporary)
            tool = root / "xcrun"
            tool.write_text('''#!/usr/bin/env python3
import json,pathlib,sys
args=sys.argv[1:]
if "launch" in args:
    pathlib.Path(args[args.index("--json-output")+1]).write_text(json.dumps({"result":{"process":{"processIdentifier":1}}}))
if "copy" in args:
    token=pathlib.Path(args[args.index("--source")+1]).stem
    destination=pathlib.Path(args[args.index("--destination")+1])
    if destination.exists(): sys.exit(1)
    counter=pathlib.Path(__file__).with_name("counter")
    count=int(counter.read_text())+1 if counter.exists() else 1
    counter.write_text(str(count))
    destination.write_text(json.dumps({"scenario":"APP-MOTION-REAL","run":"stale" if count == 1 else token,"final":count != 2,"checks":{"observed":True},"counts":{"samples":1},"digests":{},"phase":1,"prompt":"tap Face ID\\nthen Camera on the iPhone","private":"must-not-export"}))
''')
            tool.chmod(0o700)
            environment = dict(os.environ, AB_JOHAN_PROMPT_FILE=str(root / "prompts.txt"), PATH=str(root) + os.pathsep + os.environ["PATH"])
            process = subprocess.run([sys.executable, str(pathlib.Path(__file__).with_name("run.py")), "APP-MOTION-REAL", "--device", "test", "--locked"], env=environment, capture_output=True, text=True, timeout=10)
            self.assertEqual(process.returncode, 0, process.stderr)
            self.assertEqual(json.loads(process.stdout), {"checks": {"observed": True}, "counts": {"samples": 1}, "digests": {}})
            self.assertNotIn("must-not-export", process.stdout)
            self.assertEqual((root / "counter").read_text(), "3")
            prompts = (root / "prompts.txt").read_text().splitlines()
            self.assertEqual(len(prompts), 3)
            self.assertTrue(all(line[2] == ":" and "iPhone" in line for line in prompts))
            self.assertEqual(sum("tap Face ID then Camera" in line for line in prompts), 1)


if __name__ == "__main__":
    unittest.main()
