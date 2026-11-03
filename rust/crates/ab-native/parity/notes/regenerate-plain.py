import hashlib
import json
import pathlib
import subprocess
import sys

path=pathlib.Path(__file__).with_name('plain.jsonl')
fixtures=path.resolve().parents[5]/'lab/fixtures/notes'
result=subprocess.check_output([str(pathlib.Path.home()/'bin/ab-local'),sys.argv[1],str(fixtures)],text=True)
rows=[]
for line in result.splitlines():
    value=json.loads(line)
    if 'text' in value:
        text=value['text'];value['utf8_sha256']=hashlib.sha256(text.encode()).hexdigest()
        if len(text)>10000:del value['text']
    rows.append(value)
path.write_text(''.join(json.dumps(value,ensure_ascii=False,separators=(',',':'))+'\n' for value in rows))
