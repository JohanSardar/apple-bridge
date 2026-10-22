import { test, expect, type BrowserContext, type Page } from '@playwright/test';
import { control } from './session';
let cookies: Parameters<BrowserContext['addCookies']>[0] = [];
let demoFixtures: Record<string,string> = {};
type ScopedSession = {url:string;cookie:Parameters<BrowserContext['addCookies']>[0][number]};
let scopedReplica: ScopedSession;
test.beforeAll(async ({request}) => { cookies = [(await control(request, '/session')).cookie]; demoFixtures = await control(request,'/demo-fixtures'); scopedReplica = await control(request,'/replica-session?ready=serving'); });
async function installSession(page:Page, cookie:ScopedSession['cookie']) {
    const origin=new URL(cookie.url!).origin;
    expect(cookie.name).toBe(new URL(origin).protocol==='http:' ? 'ab_session_local' : '__Host-ab_session');
    if(new URL(origin).protocol==='http:')expect(cookie.secure).toBe(false);
    await page.goto(origin);
    await expect(page.getByRole('heading',{name:'Sign in with a passkey',exact:true})).toBeVisible();
    await page.context().addCookies([cookie]);
    const stored=await page.context().cookies(origin);
    expect(stored.some(value => value.name===cookie.name && value.value===cookie.value && value.secure===cookie.secure && value.sameSite===cookie.sameSite)).toBe(true);
}
test.beforeEach(async ({page}) => { await installSession(page,cookies[0]); });
async function captureNewPlate(page:Page, project:string, state:string) {
    if(!project.startsWith('chromium-'))return;
    await expect(page.locator('main.transcript')).toHaveCSS('opacity','1');
    await page.screenshot({path:`../lab/evidence/private/web/compose-new-${state}-${project}.png`,animations:'disabled'});
}
test('WEB-MSG-NEW-TOKENS keyboard and pointer autocomplete tokenize recipients and load an existing read-only chat', async ({page}, info) => {
    const ids = demoFixtures, sent: any[] = [], received: any[] = [];
    page.on('websocket', socket => {
        socket.on('framesent', event => { try { sent.push(JSON.parse(String(event.payload))); } catch {} });
        socket.on('framereceived', event => { try { const frame=JSON.parse(String(event.payload)); received.push({at:Date.now(),t:frame.t,id:frame.id,sub:frame.sub,cursor:frame.cursor,items:frame.data?.items?.length,records:frame.data?.records?.length,chats:frame.data?.chats?.length,changes:frame.changes?.length,code:frame.code}); } catch {} });
    });
    await page.goto('/messages');
    const modifier=await page.evaluate(() => /Mac|iPhone|iPad/.test(navigator.platform) ? 'Meta' : 'Control');
    await page.keyboard.press(modifier+'+n');
    await expect(page).toHaveURL(/\/messages\/new$/);
    const to = page.getByRole('combobox', {name:'To', exact:true});
    await expect(to).toBeFocused();
    await to.fill('Beryl01');
    await expect(page.getByRole('option').first()).toContainText('Demo Beryl01 Example');
    await expect(page.getByRole('option')).toHaveCount(2);
    await captureNewPlate(page,info.project.name,'lookup');
    await to.press('ArrowUp');
    await expect(to).toHaveAttribute('aria-activedescendant','recipient-option-1');
    await to.press('Escape'); await to.press('ArrowDown'); await to.press('Enter');
    await expect(page.getByRole('list', {name:'Recipients'}).getByRole('listitem')).toHaveCount(1);
    await expect(page.getByRole('region', {name:'Existing conversation'})).toHaveAttribute('data-chat-id', ids['demo-chat-01']);
    await expect(page.locator('.messages-transcript')).toBeVisible();
    await expect(page.getByRole('status', {name:'Message service'})).toContainText('SMS');
    try { await expect(page.locator('.message-row').first()).toBeVisible(); }
    catch (error) {
        const initial={subscriptions:sent.filter(frame => frame.t==='sub').map(frame => ({id:frame.id,view:frame.view,chat_id:frame.args?.chat_id})),received:received.slice(-40)};
        await page.goto('/messages/'+encodeURIComponent(ids['demo-chat-01']));
        const directLoaded=await page.locator('.message-row').first().waitFor({state:'visible',timeout:6000}).then(() => true,() => false);
        await info.attach('new-history-baseline-diagnostic',{contentType:'application/json',body:JSON.stringify({initial,direct_existing_loaded:directLoaded,subsequent:received.slice(-40),writes:sent.filter(frame => frame.t==='cmd').length})});
        throw error;
    }
    await captureNewPlate(page,info.project.name,'history');
    expect(sent.filter(frame => frame.t === 'cmd')).toHaveLength(0);
    expect(sent.filter(frame => frame.t === 'sub' && frame.view === 'thread').some(frame => frame.args.chat_id === ids['demo-chat-01'])).toBe(true);
    await page.getByRole('button', {name:/Remove Demo Beryl01/}).click();
    await to.fill('Cobalt02');
    await page.getByRole('option').filter({hasText:'mobile'}).click();
    await expect(page.getByRole('list', {name:'Recipients'})).toContainText('Demo Cobalt02 Example');
    await expect(page.getByRole('status', {name:'Message service'})).toContainText('RCS');
    await page.getByRole('button', {name:/Remove Demo Cobalt02/}).click();
    await to.fill('person01@');
    await expect(page.getByRole('option')).toHaveCount(1);
    await expect(page.getByRole('option')).toContainText('demo.person01@example.com');
    await to.press('Enter');
    await expect(page.getByRole('list', {name:'Recipients'}).getByRole('listitem')).toHaveAttribute('data-recipient-value','demo.person01@example.com');
    expect(sent.filter(frame => frame.t === 'cmd')).toHaveLength(0);
});
test('WEB-MSG-NEW-GROUP raw handles tokenize once, new groups stay disabled and an exact existing group loads', async ({page}, info) => {
    const ids=demoFixtures;
    await page.goto('/messages/new');
    const to = page.getByRole('combobox', {name:'To', exact:true});
    await to.fill('first.recipient@example.com'); await to.press('Enter');
    await to.fill('FIRST.RECIPIENT@example.com'); await to.press(',');
    await expect(page.getByRole('list', {name:'Recipients'}).getByRole('listitem')).toHaveCount(1);
    await to.fill('second.recipient@example.com'); await to.press('Tab');
    await expect(page.getByRole('list', {name:'Recipients'}).getByRole('listitem')).toHaveCount(2);
    await expect(page.getByText("New group messages aren't available yet", {exact:true})).toBeVisible();
    await page.getByRole('textbox', {name:'Message', exact:true}).fill('[ab-test new] group stays disabled');
    await expect(page.locator('button[aria-label="Send message"]:enabled')).toHaveCount(0);
    await to.focus(); await to.press('Backspace');
    await expect(page.getByRole('list', {name:'Recipients'}).getByRole('listitem')).toHaveCount(1);
    await expect(page.getByText("New group messages aren't available yet", {exact:true})).toHaveCount(0);
    await page.getByRole('button',{name:'Remove first.recipient@example.com',exact:true}).click();
    for(const query of ['Aster00','Beryl01','Cobalt02']) {
        await to.fill(query); await page.getByRole('option').filter({hasText:'mobile'}).click();
        if(query==='Beryl01')await expect(page.getByText("New group messages aren't available yet",{exact:true})).toBeVisible();
    }
    await expect(page.getByRole('region',{name:'Existing conversation'})).toHaveAttribute('data-chat-id',ids['demo-chat-00']);
    await expect(page.locator('.messages-transcript')).toBeVisible();
    await expect(page.getByRole('button',{name:'Send message',exact:true})).toBeDisabled();
    await expect(page.getByRole('button',{name:'Send message',exact:true})).toHaveCSS('opacity','0.4');
    await expect(page.locator('.message-row').first()).toBeVisible();
    await captureNewPlate(page,info.project.name,'group');
});
test.describe('Recipient response ordering fault injection', () => {
test.use({serviceWorkers:'block'});
test('WEB-MSG-NEW-LATEST lookup debounces, cancels stale requests and only displays the latest query', async ({page}) => {
    const calls: {q:string;at:number}[] = [];
    let release: (() => Promise<void>) | undefined;
    await page.addInitScript(() => {
        const original = window.fetch.bind(window);
        window.fetch = (input, init) => {
            if (String(input).endsWith('/web/v1/lookup') && typeof init?.body === 'string' && JSON.parse(init.body).q === 'Aster00') init.signal?.addEventListener('abort', () => { (window as any).__lookupAborted = true; });
            return original(input, init);
        };
    });
    await page.route('**/web/v1/lookup', async route => {
        const q = route.request().postDataJSON().q;
        calls.push({q,at:Date.now()});
        if (q === 'Aster00') { await new Promise<void>(resolve => { release = async () => { try { await route.fulfill({json:{capability:'available',results:[{contact_id:'old-contact',display_name:'STALE RESULT',handles:[{value:'old@example.com',kind:'email',label:'home'}]}]}}); } catch {} resolve(); }; }); return; }
        const response = await route.fetch(); await route.fulfill({response});
    });
    await page.goto('/messages/new');
    const to = page.getByRole('combobox', {name:'To', exact:true});
    await to.evaluate(element => {
        const input = element as HTMLInputElement;
        for (const value of ['Ast','Aster00']) { input.value=value; input.dispatchEvent(new InputEvent('input',{bubbles:true,data:value,inputType:'insertText'})); }
    });
    await expect.poll(() => calls.length).toBe(1);
    expect(calls[0].q).toBe('Aster00');
    await to.fill('Beryl01');
    await expect(page.getByRole('option').first()).toContainText('Demo Beryl01 Example');
    await expect.poll(() => page.evaluate(() => (window as any).__lookupAborted === true)).toBe(true);
    await release!();
    await expect(page.getByText('STALE RESULT', {exact:true})).toHaveCount(0);
    expect(calls.map(call => call.q)).toEqual(['Aster00','Beryl01']);
});
});
test('WEB-MSG-NEW-SEND unknown recipient creates no empty chat, sends once and survives reconnect without redispatch', async ({page, request}, info) => {
    const recipient='new.'+info.project.name+'@example.com';
    const fixtures = demoFixtures;
    const before = new Set(Object.entries(fixtures).filter(([key]) => key.startsWith('demo-chat-')).map(([,value]) => value));
    const sent: any[] = [], statuses: any[] = [], chatRows = new Map<string, any>(), historySubs=new Set<string>();
    let historyFrames=0;
    let close: (() => void) | undefined, welcomes = 0, holdObserved = true, generation = 0, releaseWelcome: (() => void) | undefined;
    await page.routeWebSocket('**/web/v1/live', socket => {
        const server = socket.connectToServer(), connection = ++generation; close = () => { void socket.close({code:4000,reason:'fixture_reconnect'}); };
        socket.onMessage(data => { const frame=JSON.parse(String(data)); sent.push(frame); if(frame.t==='sub' && frame.view==='chats' && frame.args.participants?.[0]===recipient)historySubs.add(frame.id); server.send(data); });
        server.onMessage(data => { const frame = JSON.parse(String(data)); if(frame.t === 'welcome') { welcomes++; if(connection > 1) { releaseWelcome = () => socket.send(data); return; } } if(frame.t === 'cmd_status') statuses.push(frame); if(historySubs.has(frame.sub)) { historyFrames++; for(const row of frame.data?.chats ?? [])chatRows.set(row.chat.id,row); for(const change of frame.changes ?? [])if(change.row?.chat)chatRows.set(change.row.chat.id,change.row); } if(holdObserved && frame.command?.state === 'observed') return; socket.send(data); });
    });
    await page.goto('/messages/new');
    const to = page.getByRole('combobox', {name:'To', exact:true});
    await to.fill(recipient); await to.press('Enter');
    await expect(page.getByRole('status', {name:'Message service'})).toContainText('Unknown');
    await expect.poll(() => historyFrames).toBeGreaterThan(0);
    expect(chatRows.size).toBe(0);
    await expect(page.getByRole('region', {name:'Existing conversation'})).toHaveCount(0);
    expect(sent.filter(frame => frame.t === 'cmd')).toHaveLength(0);
    await page.getByRole('textbox', {name:'Message', exact:true}).fill('[ab-test new] one explicit send');
    const send = page.getByRole('button', {name:'Send message', exact:true});
    await send.evaluate(button => button.dispatchEvent(new MouseEvent('click', {bubbles:true})));
    await page.evaluate(() => new Promise(requestAnimationFrame));
    expect(sent.filter(frame => frame.t === 'cmd')).toHaveLength(0);
    await send.click();
    await expect.poll(() => statuses.some(frame => frame.command?.state === 'observed')).toBe(true);
    await expect.poll(() => [...chatRows.keys()].some(id => !before.has(id))).toBe(true);
    await expect(page).toHaveURL(/\/messages\/new$/);
    await expect(to).toBeDisabled();
    await expect(page.getByRole('button', {name:'Remove '+recipient, exact:true})).toBeDisabled();
    await expect(page.getByRole('region', {name:'Existing conversation'})).toHaveCount(0);
    const command = sent.find(frame => frame.t === 'cmd');
    expect(command.op).toBe('messages.send');
    expect(command.args.handles).toEqual([recipient]);
    expect(command.args.chat_id ?? '').toBe('');
    close!();
    await expect(page.getByRole('status').filter({hasText:'Not sure if delivered'}).first()).toBeVisible();
    await expect.poll(() => Boolean(releaseWelcome)).toBe(true);
    holdObserved = false; releaseWelcome!();
    await expect(page).toHaveURL(/\/messages\/cht_/);
    const chatID = decodeURIComponent(new URL(page.url()).pathname.split('/')[2]);
    expect(before.has(chatID)).toBe(false);
    expect([...chatRows.keys()]).toEqual([chatID]);
    await expect(page.locator('.messages-transcript')).toBeVisible();
    const observedID=statuses.find(frame => frame.command?.state==='observed')?.command?.observed_record_ids?.[0];
    expect(observedID).toBeTruthy();
    await expect(page.locator('.message-row[data-message="'+observedID+'"]')).toHaveCount(1);
    await expect(page.locator('.new-pending-row')).toHaveCount(0);
    await expect.poll(() => welcomes).toBeGreaterThan(1);
    expect(sent.filter(frame => frame.t === 'resolve')).toHaveLength(0);
    expect(sent.some(frame => frame.t === 'sub' && frame.args?.collection === 'system.command' && frame.args.id === command.id)).toBe(true);
    expect(sent.filter(frame => frame.t === 'cmd').map(frame => frame.id)).toEqual([command.id]);
    const effect = await request.post(process.env.AB_WEB_CONTROL!+'/command-effect',{headers:{'X-Bridge-Lab':process.env.AB_WEB_CONTROL_KEY!},data:{command_id:command.id}});
    expect(effect.ok()).toBe(true); expect((await effect.json()).count).toBe(1);
});
test('WEB-MSG-NEW-LOOKUP returns bounded projected recipient fields with no-store and validates query limits', async ({page, request}) => {
    await page.goto('/messages/new');
    const unauthorized = await request.post(new URL('/web/v1/lookup',page.url()).href,{headers:{Origin:new URL(page.url()).origin},data:{q:'Demo'}});
    expect(unauthorized.status()).toBe(401);
    const lookup = (body: unknown) => page.request.post('/web/v1/lookup',{headers:{Origin:new URL(page.url()).origin},data:body});
    const response = await lookup({q:'Demo',limit:3});
    expect(response.status()).toBe(200); expect(response.headers()['cache-control']).toBe('no-store');
    const value = await response.json(); expect(value.capability).toBe('available'); expect(value.results).toHaveLength(3);
    for(const row of value.results) { expect(Object.keys(row).sort()).toEqual(['contact_id','display_name','handles']); for(const handle of row.handles)expect(Object.keys(handle).sort()).toEqual(['kind','label','value']); }
    const partial = await lookup({q:'person01@'}); expect((await partial.json()).results).toHaveLength(1);
    for(const body of [{q:''},{q:' '.repeat(5)},{q:'x'.repeat(65)},{q:'Demo',limit:21},{q:'Demo',limit:0}])expect((await lookup(body)).status()).toBe(400);
});
test('WEB-MSG-NEW-SCOPE a device without contacts scope gets no results and retains raw handle entry', async ({browser}, info) => {
    const replica = scopedReplica;
    const context = await browser.newContext({viewport:info.project.use.viewport,colorScheme:info.project.use.colorScheme,hasTouch:info.project.use.hasTouch});
    try {
        const page = await context.newPage();
        await installSession(page,replica.cookie);
        await page.goto(replica.url+'/messages/new');
        const response = await page.request.post(replica.url+'/web/v1/lookup',{headers:{Origin:replica.url},data:{q:'Demo'}});
        expect(response.status()).toBe(200);
        expect(await response.json()).toEqual({results:[],capability:'unavailable'});
        const to = page.getByRole('combobox',{name:'To',exact:true});
        await to.fill('raw.recipient@example.com'); await to.press('Enter');
        await expect(page.getByRole('list',{name:'Recipients'})).toContainText('raw.recipient@example.com');
        await expect(page.getByRole('option')).toHaveCount(0);
    } finally { await context.close(); }
});

test('WEB-MSG-NEW-HISTORY unavailable history stays quiet and never blocks an explicit send', async ({page, request}, info) => {
    const recipient='history.unavailable.'+info.project.name+'@example.com';
    const held = new Set<string>(), writes: any[] = [], statuses: any[] = [];
    let blockedFrames=0;
    await page.routeWebSocket('**/web/v1/live', socket => {
        const server = socket.connectToServer();
        socket.onMessage(data => { const frame=JSON.parse(String(data)); if(frame.t==='sub' && frame.view==='chats' && frame.args.participants)held.add(frame.id); if(frame.t==='cmd')writes.push(frame); server.send(data); });
        server.onMessage(data => { const frame=JSON.parse(String(data)); if(frame.t==='cmd_status')statuses.push({id:frame.id,status:frame.status,state:frame.command?.state,disposition:frame.command?.disposition,commit_seq:frame.command?.commit_seq,record_count:frame.command?.observed_record_ids?.length}); if(held.has(frame.sub) && ['snap','delta'].includes(frame.t)){blockedFrames++;return;} socket.send(data); });
    });
    await page.goto('/messages/new');
    const to=page.getByRole('combobox',{name:'To',exact:true});
    await to.fill(recipient); await to.press('Enter');
    await expect.poll(() => blockedFrames).toBeGreaterThan(0);
    await expect(page.getByText("Couldn't load earlier messages",{exact:true})).toBeVisible();
    await expect(page.getByRole('region',{name:'Existing conversation'})).toHaveCount(0);
    await page.getByRole('textbox',{name:'Message',exact:true}).fill('[ab-test new] history is optional');
    const send=page.getByRole('button',{name:'Send message',exact:true});
    await expect(send).toBeEnabled(); await send.click();
    try { await expect(page).toHaveURL(/\/messages\/cht_/); }
    catch (error) {
        const commandID=writes[0]?.id;
        const response=commandID ? await request.post(process.env.AB_WEB_CONTROL!+'/command-effect',{headers:{'X-Bridge-Lab':process.env.AB_WEB_CONTROL_KEY!},data:{command_id:commandID}}) : undefined;
        await info.attach('history-command-diagnostic',{contentType:'application/json',body:JSON.stringify({writes:writes.length,statuses,effect:response?.ok() ? await response.json() : undefined})});
        throw error;
    }
    expect(writes).toHaveLength(1); expect(writes[0].args.handles).toEqual([recipient]);
});

test('WEB-MSG-NEW-EXISTING live metadata keeps the existing transcript mounted during send', async ({page}) => {
    const ids=demoFixtures, writes:any[]=[], historySubs=new Set<string>(), metadataMessages=new Set<string>();
    let observedID='';
    await page.routeWebSocket('**/web/v1/live', socket => {
        const server=socket.connectToServer();
        socket.onMessage(data => { const frame=JSON.parse(String(data)); if(frame.t==='cmd')writes.push(frame); if(frame.t==='sub' && frame.view==='chats' && frame.args.participants)historySubs.add(frame.id); server.send(data); });
        server.onMessage(data => {
            const frame=JSON.parse(String(data));
            if(historySubs.has(frame.sub))for(const row of [...(frame.data?.chats ?? []),...(frame.changes ?? []).flatMap((change:any) => change.row?.chat ? [change.row] : [])])if(row.last_message?.id)metadataMessages.add(row.last_message.id);
            if(['observed','delivered'].includes(frame.command?.state)) { observedID=frame.command.observed_record_ids?.[0] ?? observedID; return; }
            socket.send(data);
        });
    });
    await page.goto('/messages/new');
    await page.getByRole('combobox',{name:'To',exact:true}).fill('Beryl01');
    await page.getByRole('option').filter({hasText:'mobile'}).click();
    await expect(page.getByRole('region',{name:'Existing conversation'})).toHaveAttribute('data-chat-id',ids['demo-chat-01']);
    await expect(page.locator('.message-row').first()).toBeVisible();
    await expect.poll(() => metadataMessages.size).toBeGreaterThan(0);
    const metadataBefore = new Set(metadataMessages);
    await page.locator('.messages-transcript').evaluate(element => { (window as any).__newExistingRoot=element; });
    await page.getByRole('textbox',{name:'Message',exact:true}).fill('[ab-test new] keep the existing conversation mounted');
    await page.getByRole('button',{name:'Send message',exact:true}).click();
    await expect.poll(() => observedID).not.toBe('');
    await expect(page.locator('.message-row[data-message="'+observedID+'"]')).toBeVisible();
    await expect.poll(() => [...metadataMessages].some(id => !metadataBefore.has(id))).toBe(true);
    await page.evaluate(() => new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
    await expect(page).toHaveURL(/\/messages\/new$/);
    expect(await page.locator('.messages-transcript').evaluate(element => element===(window as any).__newExistingRoot)).toBe(true);
    expect(writes).toHaveLength(1); expect(writes[0].args.chat_id).toBe(ids['demo-chat-01']);
});
