import argparse
import pathlib
import shutil
import subprocess

parser = argparse.ArgumentParser()
parser.add_argument('output')
parser.add_argument('support')
parser.add_argument('decoders')
parser.add_argument('sources', nargs='*', choices=['telephony', 'voice-memos', 'mail', 'notes'])
args = parser.parse_args()
parity = pathlib.Path(__file__).resolve().parent.parent
output = pathlib.Path(args.output).resolve()
support = pathlib.Path(args.support).resolve()
decoders = pathlib.Path(args.decoders).resolve()
output.mkdir(parents=True, exist_ok=True)
for source in args.sources or ['telephony', 'voice-memos', 'mail', 'notes']:
    if shutil.disk_usage('/').free < 8 * 1024 ** 3:
        raise SystemExit('less than 8 GiB free; refusing to build')
    script = parity / source / 'build-swift.sh'
    text = script.read_text().replace('"$HOME/bin/ab-local" ', '')
    subprocess.run(['bash', '-c', text, str(script), str(output), str(support), str(decoders)], check=True)
