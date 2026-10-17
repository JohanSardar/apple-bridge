import json
import os
import pathlib
import shutil
import signal
import subprocess
import sys
import tempfile

request = json.load(sys.stdin)
os.umask(0o077)
root = pathlib.Path(request['directory'])
root.mkdir(mode=0o700, parents=True, exist_ok=True)
auth_home = pathlib.Path(os.environ.get('CODEX_HOME', str(pathlib.Path.home() / '.codex')))
with tempfile.TemporaryDirectory(prefix='codex-', dir=root) as temporary:
    home = pathlib.Path(temporary)
    auth = auth_home / 'auth.json'
    if auth.exists():
        shutil.copyfile(auth, home / 'auth.json')
    config = '''approval_policy = "never"
web_search = "disabled"
[features]
shell_tool = false
[ mcp_servers.apple_bridge ]
url = %s
bearer_token_env_var = "APPLE_BRIDGE_AGENT_TOKEN"
startup_timeout_sec = 30
[ mcp_servers.apple_bridge.tools ]
''' % json.dumps(request['url'])
    (home / 'config.toml').write_text(config)
    environment = dict(os.environ)
    environment['CODEX_HOME'] = str(home)
    environment['APPLE_BRIDGE_AGENT_TOKEN'] = request['token']
    binary = shutil.which('codex') or str(pathlib.Path.home() / '.npm-global/bin/codex')
    command = [binary, 'exec', '--model', 'gpt-6-luna', '--sandbox', 'read-only', '--skip-git-repo-check', '--ephemeral', '--json', '--color', 'never', '-C', str(home), '-o', str(home / 'answer'), '-']
    with (root / 'transcript.jsonl').open('w') as output, (root / 'stderr.log').open('w') as error:
        process = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=output, stderr=error, env=environment, start_new_session=True)
        try:
            process.communicate(request['prompt'].encode(), timeout=180)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGTERM)
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()
    calls = 0
    forbidden = 0
    for line in (root / 'transcript.jsonl').read_text().splitlines():
        try:
            event = json.loads(line)
        except ValueError:
            continue
        item = event.get('item', {})
        if event.get('type') != 'item.completed':
            continue
        kind = item.get('type', '')
        if kind == 'mcp_tool_call':
            if item.get('server') == 'apple_bridge':
                calls += 1
            else:
                forbidden += 1
        elif kind not in ('agent_message', 'reasoning', 'todo_list'):
            forbidden += 1
    answer = (home / 'answer').read_text() if (home / 'answer').exists() else ''
    print(json.dumps({'exit_code': process.returncode, 'mcp_calls': calls, 'forbidden_calls': forbidden, 'answer': answer}))
