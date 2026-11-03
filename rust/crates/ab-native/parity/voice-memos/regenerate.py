import json
import pathlib
import subprocess
import sys
import tempfile

path=pathlib.Path(__file__).with_name('decoders.jsonl')
vectors=[{key:value for key,value in json.loads(line).items() if key in ['name','input','encoding']} for line in path.read_text().splitlines()]
with tempfile.TemporaryDirectory(prefix='ab-vmemo-codec-') as root:
    source=pathlib.Path(root)/'input.json';source.write_text(json.dumps(vectors))
    result=subprocess.check_output([str(pathlib.Path.home()/'bin/ab-local'),sys.argv[1],str(source)])
    assert len(result.splitlines())==len(vectors)
    path.write_bytes(result)
