import { readdir, readFile, writeFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
const assets = (await readdir('dist/assets')).filter(name => /\.(js|css)$/.test(name)).map(name => '/assets/' + name);
const version = createHash('sha256').update(await readFile('dist/index.html')).digest('hex').slice(0, 16);
await writeFile('dist/sw.js', (await readFile('public/sw.js', 'utf8')).replace('"__PRECACHE__"', JSON.stringify(['/','/index.html','/manifest.webmanifest','/icon.svg','/icons.svg',...assets])).replace('__VERSION__', version));
