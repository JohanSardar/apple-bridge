import { test, expect, type BrowserContext } from '@playwright/test';
import { control } from './session';
let trusted: Parameters<BrowserContext['addCookies']>[0] = [], publicSession: Parameters<BrowserContext['addCookies']>[0] = [], ids: Record<string,string>;
test.beforeAll(async ({request}) => {
    trusted = [(await control(request,'/session')).cookie];
    publicSession = [(await control(request,'/session?kind=public')).cookie];
    ids = await control(request,'/demo-fixtures');
});
const path = () => '/__fixtures/compose?chat='+encodeURIComponent(ids['message-chat'])+'&alternate='+encodeURIComponent(ids['demo-chat-01']);
test('WEB-MSG-COMPOSER trusted Enter sends once, Shift+Enter preserves a newline and the draft clears', async ({page,context}) => {
    await context.addCookies(trusted);
    const commands:any[]=[],statuses:any[]=[],held:(()=>void)[]=[];
    let holding=true;
    await page.routeWebSocket('**/web/v1/live', socket => {
        const server=socket.connectToServer(), threads=new Set<string>();
        socket.onMessage(data => {const frame=JSON.parse(String(data));if(frame.t==='cmd')commands.push(frame);if(frame.t==='sub'&&frame.view==='thread')threads.add(frame.id);server.send(data);});
        server.onMessage(data => {
            const frame=JSON.parse(String(data));
            if(frame.t==='cmd_status')statuses.push(frame);
            if(commands.length&&holding&&(['ack','cmd_status'].includes(frame.t)||frame.t==='delta'&&threads.has(frame.sub))){held.push(()=>socket.send(data));return;}
            socket.send(data);
        });
    });
    await page.goto(path());
    const field=page.getByRole('textbox',{name:'Message',exact:true}), send=page.getByRole('button',{name:'Send message',exact:true});
    await expect(field).toBeVisible();await expect(field).toHaveAttribute('placeholder','iMessage');await expect(send).not.toBeVisible();
    await field.fill('[ab-test compose] first line');await field.press('Shift+Enter');await field.pressSequentially('second line');
    await expect(field).toHaveValue('[ab-test compose] first line\nsecond line');expect(commands).toHaveLength(0);
    await expect(send).toBeEnabled();await field.press('Enter');await expect.poll(()=>commands.length).toBe(1);
    const command=commands[0];expect(command.op).toBe('messages.send');expect(command.args).toEqual({chat_id:ids['message-chat'],text:'[ab-test compose] first line\nsecond line'});
    await expect(field).toHaveValue('');await expect(page.locator('[data-local-echo]')).toHaveCount(1);
    await expect.poll(()=>statuses.some(value=>value.command?.observed_record_ids?.length)).toBe(true);
    const recordID=statuses.find(value=>value.command?.observed_record_ids?.length).command.observed_record_ids[0];
    holding=false;held.splice(0).forEach(release=>release());
    await expect(page.locator('[data-local-echo]')).toHaveCount(0);await expect(page.locator('[data-record-id="'+recordID+'"]')).toHaveCount(1);
    await field.press('Enter');expect(commands).toHaveLength(1);
});
test('WEB-MSG-COMPOSER trusted drafts stay per chat, restore after reload and the field stops at six lines', async ({page,context}) => {
    await context.addCookies(trusted);await page.goto(path());
    const field=page.getByRole('textbox',{name:'Message',exact:true});await expect(field).toBeVisible();
    const draft=Array.from({length:12},(_,index)=>'[ab-test compose] line '+index).join('\n');await field.fill(draft);
    await page.evaluate(()=>new Promise(requestAnimationFrame));
    const geometry=await field.evaluate(element=>{const style=getComputedStyle(element);return {height:element.getBoundingClientRect().height,maximum:parseFloat(style.lineHeight)*6+parseFloat(style.paddingTop)+parseFloat(style.paddingBottom)+parseFloat(style.borderTopWidth)+parseFloat(style.borderBottomWidth),scroll:element.scrollHeight,client:element.clientHeight};});
    expect(geometry.height).toBeLessThanOrEqual(geometry.maximum+1);expect(geometry.height).toBeGreaterThanOrEqual(geometry.maximum-1);expect(geometry.scroll).toBeGreaterThan(geometry.client);
    await page.getByRole('button',{name:'Switch fixture chat',exact:true}).click();await expect(field).toHaveValue('');await expect(field).toHaveAttribute('placeholder','Text Message');
    await field.fill('[ab-test compose] separate draft');await page.getByRole('button',{name:'Switch fixture chat',exact:true}).click();await expect(field).toHaveValue(draft);
    await page.reload();await expect(field).toHaveValue(draft);
});
test('WEB-MSG-COMPOSER public drafts survive chat switches only in memory', async ({page,context}) => {
    await context.addCookies(publicSession);await page.goto(path());
    const field=page.getByRole('textbox',{name:'Message',exact:true});await expect(field).toBeVisible();await field.fill('[ab-test compose] public draft');
    await page.getByRole('button',{name:'Switch fixture chat',exact:true}).click();await expect(field).toHaveValue('');
    await page.getByRole('button',{name:'Switch fixture chat',exact:true}).click();await expect(field).toHaveValue('[ab-test compose] public draft');
    expect(await page.evaluate(()=>Object.keys(localStorage).filter(key=>key.startsWith('ab.messages.draft.')))).toEqual([]);
    await page.reload();await expect(field).toHaveValue('');
});
test('WEB-MSG-COMPOSER waits for a coherent capture before Send and rejects oversized text locally', async ({page,context}) => {
    await context.addCookies(trusted);
    const held:(()=>void)[]=[],commands:any[]=[];let holding=true,proven=false;
    await page.routeWebSocket('**/web/v1/live',socket=>{
        const server=socket.connectToServer(),threads=new Set<string>();
        socket.onMessage(data=>{const frame=JSON.parse(String(data));if(frame.t==='sub'&&frame.view==='thread')threads.add(frame.id);if(frame.t==='cmd')commands.push(frame);server.send(data);});
        server.onMessage(data=>{
            const frame=JSON.parse(String(data));
            if(frame.t==='snap'&&frame.data?.records?.some((record:any)=>record.collection==='system.source'&&record.data?.capabilities?.includes('messages.send')))proven=true;
            if(holding&&threads.has(frame.sub)){held.push(()=>socket.send(data));return;}
            socket.send(data);
        });
    });
    await page.goto(path());const field=page.getByRole('textbox',{name:'Message',exact:true}),send=page.getByRole('button',{name:'Send message',exact:true});
    await expect(field).toBeVisible();await field.fill('[ab-test compose] wait for baseline');
    await expect.poll(()=>proven&&held.length>0).toBe(true);await expect(send).not.toBeVisible();await field.press('Enter');expect(commands).toHaveLength(0);
    holding=false;held.splice(0).forEach(release=>release());await expect(send).toBeEnabled();
    await field.fill('x'.repeat(65537));await expect(send).toBeDisabled();await expect(page.getByRole('alert')).toHaveText('Message is too long');await field.press('Enter');expect(commands).toHaveLength(0);
});
test('WEB-MSG-COMPOSER signing out removes trusted drafts', async ({page,context}) => {
    await context.addCookies(trusted);await page.goto(path());
    const field=page.getByRole('textbox',{name:'Message',exact:true});await expect(field).toBeVisible();await field.fill('[ab-test compose] remove on sign out');
    await expect.poll(()=>page.evaluate(()=>Object.keys(localStorage).filter(key=>key.startsWith('ab.messages.draft.')).length)).toBeGreaterThan(0);
    await page.getByRole('button',{name:'Sign out',exact:true}).click();await expect(field).toHaveCount(0);
    expect(await page.evaluate(()=>Object.keys(localStorage).filter(key=>key.startsWith('ab.messages.draft.')))).toEqual([]);
});
