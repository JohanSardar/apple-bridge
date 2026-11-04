import os
import subprocess
import sys

output = sys.argv[1]
mode = sys.argv[2]
families = sys.argv[3:] or sorted(
    name for name in os.listdir("lab/scenarios") if os.path.isdir(os.path.join("lab/scenarios", name))
)
os.makedirs(output, exist_ok=True)
environment = dict(os.environ, AB_LAB_SESSION_NATIVE=mode)
for family in families:
    log = os.path.join(output, "%s-%s.log" % (mode, family))
    with open(log, "w") as handle:
        try:
            subprocess.run(["packaging/dev/rust-lab.sh", "sim", "lab/scenarios/" + family], env=environment, stdout=handle, stderr=subprocess.STDOUT, timeout=2400)
        except subprocess.TimeoutExpired:
            handle.write("\nTIMEOUT\n")
    print(family, mode, flush=True)
