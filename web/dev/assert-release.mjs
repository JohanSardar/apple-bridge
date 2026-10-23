import { readFile, readdir, writeFile } from 'node:fs/promises';
import { execFileSync } from 'node:child_process';

const directory = process.env.AB_WEB_RELEASE_OUTDIR || 'dist';
const manifest = JSON.parse(await readFile(`${directory}/.vite/manifest.json`, 'utf8'));
const fixtures = Object.entries(manifest).filter(([key, chunk]) =>
  /(?:^|\/)app\/fixtures\/|Fixture(?:-[^/]*)?\.js$/u.test(key) ||
  /Fixture(?:-[^/]*)?\.js$/u.test(chunk.file));
if (fixtures.length) throw new Error(`Release contains fixture modules: ${fixtures.map(([key]) => key).join(', ')}`);
const assets = await readdir(`${directory}/assets`);
if (assets.some(file => /Fixture(?:-[^/]*)?\.js$/u.test(file))) throw new Error('Release contains fixture asset');
const commit = execFileSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' }).trim();
await writeFile(`${directory}/release-build.json`, JSON.stringify({ commit }) + '\n');
