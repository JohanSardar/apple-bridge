import { spawnSync } from 'node:child_process';
import { readdirSync, readFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

export function unmappedTitles(report, ids) {
  const failures = [];
  function visit(suite) {
    for (const spec of suite.specs ?? []) {
      const token = spec.title.match(/^([\w-]+)/u)?.[1];
      if (token && /^[A-Z][A-Z0-9]*-/u.test(token) && !ids.has(token)) {
        failures.push(`${spec.file}:${spec.line}: ${spec.title}`);
      }
    }
    for (const child of suite.suites ?? []) visit(child);
  }
  visit(report);
  return failures;
}

function manifests(directory) {
  return readdirSync(directory, { withFileTypes: true }).flatMap(entry => {
    const path = join(directory, entry.name);
    return entry.isDirectory() ? manifests(path) : /\.ya?ml$/u.test(entry.name) ? [path] : [];
  });
}

function main() {
  const web = resolve(dirname(fileURLToPath(import.meta.url)), '..');
  const ids = new Set(manifests(join(web, '../lab/scenarios')).map(path => readFileSync(path, 'utf8').match(/^id:\s*(\S+)/mu)?.[1]).filter(Boolean));
  const listed = spawnSync('pnpm', ['exec', 'playwright', 'test', '--list', '--reporter=json'], {
    cwd: web,
    env: { ...process.env, AB_WEB_PROJECTS: 'chromium-light-desktop' },
    encoding: 'utf8',
    maxBuffer: 32 * 1024 * 1024,
  });
  if (listed.error || listed.status !== 0) throw new Error(`Playwright discovery failed: ${listed.error ?? (listed.stderr || listed.stdout)}`);
  const report = JSON.parse(listed.stdout);
  const failures = unmappedTitles(report, ids);
  if (failures.length) throw new Error(`Playwright titles have no manifest ID; separate subflow names from the owning ID with whitespace:\n${failures.join('\n')}`);
  console.log('Every Playwright scenario title maps to a manifest ID.');
}

if (resolve(process.argv[1] ?? '') === fileURLToPath(import.meta.url)) main();
