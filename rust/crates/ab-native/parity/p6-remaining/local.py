import argparse
import pathlib
import runpy
import shlex
import subprocess
import sys
import tempfile

parser = argparse.ArgumentParser()
parser.add_argument('source', choices=['telephony', 'voice-memos', 'mail', 'notes'])
parser.add_argument('oracle')
parser.add_argument('reader')
parser.add_argument('data_bytes')
args = parser.parse_args()
parity = pathlib.Path(__file__).resolve().parent.parent
repository = parity.parents[3]
original_popen = subprocess.Popen

with tempfile.TemporaryDirectory(prefix='ab-p6-local-peer-') as peer:
    class LocalPopen(original_popen):
        def __init__(self, argv, *positional, **keywords):
            if isinstance(argv, (list, tuple)) and argv:
                argv = list(argv)
                if str(argv[0]) == str(pathlib.Path.home() / 'bin/ab-local'):
                    argv = argv[1:]
                elif argv[:2] == ['ssh', 'build']:
                    if len(argv) != 3:
                        raise ValueError('unexpected peer command')
                    command = argv[2]
                    command = command.replace('~/w/', shlex.quote(peer) + '/')
                    command = command.replace('~/targets/phase6-native/fixture-data-bytes', shlex.quote(str(pathlib.Path(args.data_bytes).resolve())))
                    command = command.replace('/home/johan/w/phase6-native', str(repository))
                    command = command.replace('stat -c %s ', 'stat -f %z ')
                    argv = ['/bin/zsh', '-c', command]
            super().__init__(argv, *positional, **keywords)
    subprocess.Popen = LocalPopen
    sys.argv = [str(parity / args.source / 'compare.py'), str(pathlib.Path(args.oracle).resolve()), str(pathlib.Path(args.reader).resolve())]
    runpy.run_path(sys.argv[0], run_name='__main__')
