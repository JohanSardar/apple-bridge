import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { createInterface } from 'node:readline';
import { chromium } from '@playwright/test';
import assert from 'node:assert/strict';

for (const huge of [false, true]) {
  const started = performance.now();
  const child = spawn('bin/bridge-lab', ['demo', '--seed', '17', ...(huge ? ['--huge', '--alive'] : [])], { cwd: '..', stdio: ['ignore', 'pipe', 'inherit'] });
  const exited = once(child, 'exit');
  const lines = createInterface({ input: child.stdout });
  let browser;
  let timer;
  try {
    const metadata = await Promise.race([
      once(lines, 'line').then(([line]) => JSON.parse(line)),
      exited.then(() => { throw new Error('Demo exited before readiness'); }),
      new Promise((_, reject) => { timer = setTimeout(() => reject(new Error('Demo readiness timed out')), 300000); })
    ]);
    clearTimeout(timer);
    const readyMs = performance.now() - started;
    assert.equal(metadata.counts['messages.message'], huge ? 104900 : 5000);
    browser = await chromium.launch();
    const page = await browser.newPage();
    await page.goto(metadata.url);
    assert.equal((await page.request.get(new URL('/web/v1/session', metadata.url).href)).status(), 200);
    const origin = new URL(metadata.url).origin;
    await page.goto(origin + '/__fixtures/live');
    await page.getByRole('status', { name: 'Connection' }).filter({ hasText: 'connected' }).waitFor();
    const thread = await page.evaluate(() => new Promise((resolve, reject) => {
      const socket = new WebSocket(location.origin.replace(/^http/, 'ws') + '/web/v1/live', 'web.1');
      const timeout = setTimeout(() => { socket.close(); reject(new Error('Read model timed out')); }, 20000);
      socket.onopen = () => socket.send(JSON.stringify({ t: 'hello', v: 1 }));
      socket.onmessage = event => {
        const frame = JSON.parse(event.data);
        if (frame.t === 'welcome') socket.send(JSON.stringify({ t: 'sub', id: 'chats', view: 'chats', args: { limit: 200 } }));
        if (frame.t === 'snap' && frame.sub === 'chats') {
          const chat = frame.data.chats.find(value => value.title === 'Demo Performance Archive');
          if (!chat) { clearTimeout(timeout); socket.close(); reject(new Error('Performance chat missing')); return; }
          socket.send(JSON.stringify({ t: 'sub', id: 'thread', view: 'thread', args: { chat_id: chat.chat.id, limit: 50, anchor: 'latest' } }));
        }
        if (frame.t === 'snap' && frame.sub === 'thread') {
          clearTimeout(timeout);
          socket.close();
          resolve({ count: frame.data.items.length + frame.data.older + frame.data.newer, page: frame.data.items.length });
        }
        if (frame.t === 'error') { clearTimeout(timeout); socket.close(); reject(new Error('Read model error: ' + frame.code)); }
      };
      socket.onerror = () => { clearTimeout(timeout); reject(new Error('WebSocket failed')); };
    }));
    assert.equal(thread.count, huge ? 100000 : 100);
    assert.equal(thread.page, 50);
    if (huge) {
      const before = Number(await page.getByLabel('Unread count', { exact: true }).textContent());
      await page.waitForFunction(value => Number(document.querySelector('[aria-label="Unread count"]').textContent) > value, before, { timeout: 10000 });
    }
    console.log(JSON.stringify({ mode: huge ? 'huge_alive' : 'default', counts: metadata.counts, startup_ms: metadata.startup_ms, command_to_url_ms: readyMs, thread, passed: true }));
  } finally {
    clearTimeout(timer);
    lines.close();
    await browser?.close();
    child.kill('SIGTERM');
    await exited;
  }
}
