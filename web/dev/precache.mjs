import { readFile, writeFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
const html = await readFile('dist/index.html', 'utf8');
const manifest = JSON.parse(await readFile('dist/.vite/manifest.json', 'utf8'));
const assets = new Set(), visited = new Set();
function include(key) {
  if (visited.has(key)) return;
  visited.add(key);
  const chunk = manifest[key];
  assets.add('/' + chunk.file);
  for (const css of chunk.css ?? []) assets.add('/' + css);
  for (const dependency of chunk.imports ?? []) include(dependency);
}
if (!manifest['index.html']?.isEntry) throw new Error('Shell entry missing from build manifest');
include('index.html');
for (const match of html.matchAll(/<link\b[^>]*\brel="stylesheet"[^>]*\bhref="([^"]+)"/gu)) assets.add(match[1]);
const version = createHash('sha256').update(html).digest('hex').slice(0, 16);
await writeFile('dist/sw.js', (await readFile('public/sw.js', 'utf8')).replace('"__PRECACHE__"', JSON.stringify(['/','/index.html','/manifest.webmanifest','/icon.svg','/icons.svg',...assets])).replace('__VERSION__', version));
await writeFile('dist/.gitkeep', '');
