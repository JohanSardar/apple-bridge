import { test, expect } from '@playwright/test';
import { control } from './session';
import type { ChatSummary } from '../src/data/types';

test('WEB-DEMO-SESSION demo URL signs in, serves synthetic read models, and receives live capture', async ({ page, context, request }) => {
  const snapshots:ChatSummary[][]=[];page.on('websocket',socket=>socket.on('framereceived',event=>{const frame=JSON.parse(String(event.payload));if(frame.t==='snap'&&frame.data?.chats)snapshots.push(frame.data.chats);}));
  const ids = await control(request, '/demo-fixtures');
  expect(Object.keys(ids)).toHaveLength(300);
  expect(ids['message-chat']).toBeTruthy();
  expect(ids['demo-message-00-000011']).toBeTruthy();
  const url = process.env.AB_WEB_DEMO_URL!;
  await page.goto(url);
  expect(new URL(page.url()).pathname).toBe('/messages');
  const response = await page.request.get('/web/v1/session');
  expect(response.status()).toBe(200);
  const cookie = (await context.cookies()).find(value => value.name === 'ab_demo_session')!;
  expect(cookie.httpOnly && !cookie.secure && cookie.sameSite === 'Strict').toBe(true);
  await page.goto('/__fixtures/live');
  await expect(page.getByRole('status', { name: 'Connection' })).toHaveText('connected');
  await expect.poll(()=>snapshots.length).toBeGreaterThan(0);
  const excluded=['demo-chat-34','demo-chat-37','demo-chat-38'],expected=Object.entries(ids).filter(([key])=>(key==='message-chat'||/^demo-chat-\d+$/.test(key))&&!excluded.includes(key)).map(([,id])=>id).sort();
  const chats=[...new Map(snapshots.flat().map(chat=>[chat.chat.id,chat])).values()];
  expect(chats.map(chat=>chat.chat.id).sort()).toEqual(expected);expect(chats.every(chat=>!chat.chat.data.is_archived&&!chat.chat.data.deleted_at)).toBe(true);
  await expect(page.getByLabel('Live count', { exact: true })).toHaveText(String(expected.length));
  const before = Number(await page.getByLabel('Unread count', { exact: true }).textContent());
  await control(request, '/incoming');
  await expect(page.getByLabel('Unread count', { exact: true })).toHaveText(String(before + 1));
  expect((await page.request.get(url)).status()).toBe(410);
});
