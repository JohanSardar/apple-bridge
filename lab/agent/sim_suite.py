import argparse
import json
import os
import pathlib
import secrets
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import tomllib
import urllib.error
import urllib.request


class Mcp:
    def __init__(self, url, token):
        self.url = url
        self.token = token
        self.id = 0
        self.rpc('initialize', {'protocolVersion': '2025-06-18', 'capabilities': {}, 'clientInfo': {'name': 'sim-suite-grader', 'version': '1'}})

    def rpc(self, method, params):
        self.id += 1
        body = json.dumps({'jsonrpc': '2.0', 'id': self.id, 'method': method, 'params': params}).encode()
        request = urllib.request.Request(self.url, body, {'Authorization': 'Bearer ' + self.token, 'Content-Type': 'application/json', 'Accept': 'application/json, text/event-stream', 'MCP-Protocol-Version': '2025-06-18'})
        try:
            with urllib.request.urlopen(request, timeout=60) as response:
                raw = response.read().decode()
        except urllib.error.HTTPError as error:
            raw = error.read().decode()
        if raw.startswith(('data:', 'event:')):
            raw = '\n'.join(line[5:].strip() for line in raw.splitlines() if line.startswith('data:'))
        return json.loads(raw)

    def call(self, name, arguments):
        reply = self.rpc('tools/call', {'name': name, 'arguments': arguments})
        result = reply.get('result') or {}
        return result.get('structuredContent') or {}

    def tools_list_bytes(self):
        reply = self.rpc('tools/list', {})
        return len(json.dumps(reply['result'], separators=(',', ':'), ensure_ascii=False))


def reminders(grader):
    found = grader.call('apple_read', {'operation': 'reminders.items', 'arguments': {'smart': 'all'}})
    items = []
    for section in found.get('sections', []):
        for item in section.get('items', []):
            items.append(item['reminder'])
    completed = grader.call('apple_read', {'operation': 'reminders.items', 'arguments': {'smart': 'completed'}})
    for section in completed.get('sections', []):
        for item in section.get('items', []):
            items.append(item['reminder'])
    return items


def messages_with(grader, words):
    found = grader.call('apple_read', {'operation': 'messages.search', 'arguments': {'text': words, 'limit': 20}})
    return found.get('items', [])


def notes(grader):
    found = grader.call('apple_read', {'operation': 'records.query', 'arguments': {'collection': 'notes.note', 'limit': 200}})
    return found.get('records', [])


def tasks(tag):
    def reminder_created(grader, answer):
        match = [r for r in reminders(grader) if r['data'].get('title', '').startswith(f'{tag} oat milk')]
        return bool(match) and match[0]['data'].get('due', '').startswith('2026-11-12'), {'found': len(match)}

    def setup_complete(grader):
        lists = grader.call('apple_read', {'operation': 'reminders.lists', 'arguments': {}})
        list_id = lists['lists'][0]['list']['id']
        grader.call('apple_change', {'operation': 'reminders.create', 'arguments': {'list_id': list_id, 'title': f'{tag} call the plumber'}})

    def reminder_completed(grader, answer):
        match = [r for r in reminders(grader) if r['data'].get('title') == f'{tag} call the plumber']
        return bool(match) and match[0]['data'].get('completed') is True, {'found': len(match)}

    def setup_move(grader):
        lists = grader.call('apple_read', {'operation': 'reminders.lists', 'arguments': {}})
        list_id = lists['lists'][0]['list']['id']
        grader.call('apple_change', {'operation': 'reminders.create', 'arguments': {'list_id': list_id, 'title': f'{tag} renew passport', 'due': '2026-11-15', 'due_all_day': True}})

    def reminder_moved(grader, answer):
        match = [r for r in reminders(grader) if r['data'].get('title') == f'{tag} renew passport']
        return bool(match) and match[0]['data'].get('due', '').startswith('2026-11-20'), {'found': len(match)}

    def message_sent(grader, answer):
        sent = [m for m in messages_with(grader, f'{tag} running late') if m['data'].get('is_from_me')]
        return len(sent) == 1, {'sent': len(sent)}

    def not_sent_to_ambiguous(grader, answer):
        sent = messages_with(grader, f'{tag} are you free')
        named = 'Kovac' in answer and 'Novak' in answer
        return not sent and named, {'sent': len(sent), 'named_both': named}

    def note_created(grader, answer):
        match = [n for n in notes(grader) if n['data'].get('title') == f'{tag} packing']
        text = match[0]['data'].get('plain_text', '') if match else ''
        return bool(match) and 'passport' in text and 'charger' in text, {'found': len(match)}

    def calendar_answer(grader, answer):
        return 'main street 1' in answer.lower(), {}

    def message_answer(grader, answer):
        return 'unicode' in answer.lower(), {}

    return [
        {'id': 'reminder-add', 'prompt': f'Remind me to buy oat milk on 2026-11-12 in my Bridge Test reminders list. Start the title with "{tag} oat milk".', 'grade': reminder_created},
        {'id': 'reminder-complete', 'setup': setup_complete, 'prompt': f'Mark my reminder "{tag} call the plumber" as done.', 'grade': reminder_completed},
        {'id': 'reminder-move', 'setup': setup_move, 'prompt': f'Move the due date of my reminder "{tag} renew passport" to 2026-11-20.', 'grade': reminder_moved},
        {'id': 'calendar-read', 'prompt': 'Where is my dentist appointment today? Answer with the location only.', 'grade': calendar_answer},
        {'id': 'messages-read', 'prompt': 'What did I last write in my chat with Marko Horvat? Answer with the text only.', 'grade': message_answer},
        {'id': 'messages-send', 'prompt': f'Text Ana Novak: "{tag} running late".', 'grade': message_sent},
        {'id': 'messages-ambiguous', 'prompt': f'Text Ana "{tag} are you free". If it is unclear who I mean, do not send anything and tell me the options.', 'grade': not_sent_to_ambiguous},
        {'id': 'notes-create', 'prompt': f'Create a note titled "{tag} packing" with the text "passport, charger".', 'grade': note_created},
    ]


def toml_value(value):
    if isinstance(value, dict):
        return '{' + ', '.join(f'{json.dumps(k)} = {toml_value(v)}' for k, v in value.items()) + '}'
    if isinstance(value, list):
        return '[' + ', '.join(toml_value(v) for v in value) + ']'
    if isinstance(value, bool):
        return 'true' if value else 'false'
    return json.dumps(value)


def run_codex(task, url, token, directory, model):
    source = pathlib.Path(os.environ.get('CODEX_HOME', str(pathlib.Path.home() / '.codex')))
    user = tomllib.loads((source / 'config.toml').read_text()) if (source / 'config.toml').exists() else {}
    home = pathlib.Path(tempfile.mkdtemp(prefix='codex-', dir=directory))
    if (source / 'auth.json').exists():
        shutil.copyfile(source / 'auth.json', home / 'auth.json')
    lines = []
    for key in ('model_provider', 'model_catalog_json', 'model', 'model_reasoning_effort', 'service_tier'):
        if key in user:
            lines.append(f'{key} = {toml_value(user[key])}')
    if model:
        lines = [line for line in lines if not line.startswith('model =')] + [f'model = {json.dumps(model)}']
    lines += ['approval_policy = "never"', 'web_search = "disabled"', '[features]', 'shell_tool = false', 'apps = false', 'plugins = false']
    for name, provider in user.get('model_providers', {}).items():
        lines.append(f'[model_providers.{json.dumps(name)}]')
        lines += [f'{k} = {toml_value(v)}' for k, v in provider.items()]
    lines += ['[mcp_servers.apple_bridge]', f'url = {json.dumps(url)}', 'bearer_token_env_var = "APPLE_BRIDGE_AGENT_TOKEN"', 'startup_timeout_sec = 30', 'default_tools_approval_mode = "approve"']
    (home / 'config.toml').write_text('\n'.join(lines) + '\n')
    environment = dict(os.environ, CODEX_HOME=str(home), APPLE_BRIDGE_AGENT_TOKEN=token)
    command = [shutil.which('codex') or 'codex', 'exec', '--sandbox', 'read-only', '--skip-git-repo-check', '--ephemeral', '--json', '--color', 'never', '-C', str(home), '-o', str(home / 'answer'), '-']
    transcript = directory / f'{task["id"]}.codex.jsonl'
    started = time.monotonic()
    code = run(command, task['prompt'], transcript, environment)
    elapsed = time.monotonic() - started
    calls, tools, forbidden = [], set(), 0
    for line in transcript.read_text().splitlines():
        try:
            event = json.loads(line)
        except ValueError:
            continue
        item = event.get('item', {})
        if event.get('type') != 'item.completed':
            continue
        if item.get('type') == 'mcp_tool_call' and item.get('server') == 'apple_bridge':
            calls.append(item.get('tool'))
        elif item.get('type') not in ('agent_message', 'reasoning', 'todo_list'):
            forbidden += 1
    answer = (home / 'answer').read_text() if (home / 'answer').exists() else ''
    shutil.rmtree(home, ignore_errors=True)
    return {'exit_code': code, 'calls': calls, 'forbidden_calls': forbidden, 'answer': answer, 'seconds': round(elapsed, 1)}


def run_claude(task, url, token, directory, model):
    config = directory / 'claude-mcp.json'
    config.write_text(json.dumps({'mcpServers': {'apple_bridge': {'type': 'http', 'url': url, 'headers': {'Authorization': 'Bearer ' + token}}}}))
    config.chmod(0o600)
    command = [shutil.which('claude') or 'claude', '-p', '--mcp-config', str(config), '--strict-mcp-config', '--setting-sources', '', '--output-format', 'stream-json', '--verbose', '--no-session-persistence', '--allowedTools', 'mcp__apple_bridge', '--disallowedTools', 'Bash,Edit,Write,Read,Glob,Grep,WebFetch,WebSearch,Task,NotebookEdit']
    if model:
        command += ['--model', model]
    transcript = directory / f'{task["id"]}.claude.jsonl'
    started = time.monotonic()
    code = run(command, task['prompt'], transcript, dict(os.environ))
    elapsed = time.monotonic() - started
    calls, forbidden, answer = [], 0, ''
    for line in transcript.read_text().splitlines():
        try:
            event = json.loads(line)
        except ValueError:
            continue
        if event.get('type') == 'assistant':
            for block in event.get('message', {}).get('content', []):
                if block.get('type') == 'tool_use':
                    name = block.get('name', '')
                    if name.startswith('mcp__apple_bridge__'):
                        calls.append(name.removeprefix('mcp__apple_bridge__'))
                    elif name != 'ToolSearch':
                        forbidden += 1
        if event.get('type') == 'result':
            answer = event.get('result') or ''
    return {'exit_code': code, 'calls': calls, 'forbidden_calls': forbidden, 'answer': answer, 'seconds': round(elapsed, 1)}


def run(command, prompt, transcript, environment):
    with transcript.open('w') as output, (transcript.with_suffix('.stderr')).open('w') as error:
        process = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=output, stderr=error, env=environment, start_new_session=True)
        try:
            process.communicate(prompt.encode(), timeout=300)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGTERM)
            process.wait()
    return process.returncode


def main():
    parser = argparse.ArgumentParser(description='Run real agents against a synthetic Apple Bridge world and grade the results.')
    parser.add_argument('--url', required=True)
    parser.add_argument('--agent-token-file', required=True)
    parser.add_argument('--grader-token-file', required=True)
    parser.add_argument('--agent', choices=['codex', 'claude'], required=True)
    parser.add_argument('--model', default='')
    parser.add_argument('--out', required=True)
    parser.add_argument('--only', default='')
    options = parser.parse_args()
    out = pathlib.Path(options.out)
    out.mkdir(mode=0o700, parents=True, exist_ok=True)
    private = pathlib.Path(tempfile.mkdtemp(prefix='suite-', dir=out))
    token = pathlib.Path(options.agent_token_file).read_text().strip()
    grader = Mcp(options.url, pathlib.Path(options.grader_token_file).read_text().strip())
    tag = f'[ab-test {secrets.token_hex(3)}]'
    runner = run_codex if options.agent == 'codex' else run_claude
    results = []
    for task in tasks(tag):
        if options.only and task['id'] not in options.only.split(','):
            continue
        if task.get('setup'):
            task['setup'](grader)
        outcome = runner(task, options.url, token, private, options.model)
        passed, facts = task['grade'](grader, outcome['answer'])
        passed = passed and outcome['forbidden_calls'] == 0
        results.append({'task': task['id'], 'passed': passed, 'mcp_calls': len(outcome['calls']), 'tools': outcome['calls'], 'seconds': outcome['seconds'], 'exit_code': outcome['exit_code'], 'forbidden_calls': outcome['forbidden_calls'], 'facts': facts})
        print(json.dumps(results[-1]), flush=True)
    summary = {
        'agent': options.agent,
        'model': options.model or 'default',
        'tools_list_bytes': grader.tools_list_bytes(),
        'passed': sum(r['passed'] for r in results),
        'total': len(results),
        'mcp_calls': sum(r['mcp_calls'] for r in results),
        'seconds': round(sum(r['seconds'] for r in results), 1),
        'results': results,
    }
    (out / f'{options.agent}-summary.json').write_text(json.dumps(summary, indent=2) + '\n')
    (private / 'claude-mcp.json').unlink(missing_ok=True)
    print(json.dumps({k: v for k, v in summary.items() if k != 'results'}))
    return 0 if summary['passed'] == summary['total'] else 1


if __name__ == '__main__':
    sys.exit(main())
