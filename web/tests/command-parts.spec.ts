import { test, expect, type APIRequestContext, type BrowserContext, type Page } from '@playwright/test';
import { control } from './session';
let cookies: Parameters<BrowserContext['addCookies']>[0] = [];
test.beforeAll(async ({ request }) => { cookies = [(await control(request, '/session')).cookie]; });
test.afterEach(async ({ request, page }, info) => {
    await control(request, '/message-fault?name=');
    if (!page.isClosed()) console.log('compose_parts_metric:' + JSON.stringify({ test: info.title, status: info.status, timeout_count: Number(await page.getByLabel('Projection timeout count').textContent().catch(() => '0')) }));
});
async function fixture(page: Page, context: BrowserContext, request: APIRequestContext) {
    await page.goto(new URL(cookies[0].url!).origin);
    await expect(page.getByRole('heading',{name:'Sign in with a passkey',exact:true})).toBeVisible();
    await context.addCookies(cookies);
    expect((await context.cookies(cookies[0].url!)).some(value => value.name===cookies[0].name && value.value===cookies[0].value)).toBe(true);
    const ids = await control(request, '/demo-fixtures');
    const sent: any[] = [], statuses: any[] = [], captures = new Map<string, any>();
    let baseline: any[] = [], thread = '', cursor: any, emit: (frame: any) => void = () => {}, freezeStatus = false, frozen: any[] = [];
    await page.routeWebSocket('**/web/v1/live', socket => {
        const server = socket.connectToServer(), commandViews = new Set<string>();
        emit = frame => socket.send(JSON.stringify(frame));
        socket.onMessage(data => {
            const frame = JSON.parse(String(data));
            sent.push(frame);
            if (frame.t === 'sub' && frame.view === 'thread') thread = frame.id;
            if (frame.t === 'sub' && frame.args?.collection === 'system.command') commandViews.add(frame.id);
            server.send(data);
        });
        server.onMessage(data => {
            const frame = JSON.parse(String(data));
            if (frame.t === 'cmd_status' || frame.t === 'ack') {
                if(frame.t === 'cmd_status')statuses.push(frame);
                if (freezeStatus) { frozen.push(frame); return; }
            }
            if (commandViews.has(frame.sub)) return;
            if (frame.sub === thread) {
                if (!sent.some(value => value.t === 'cmd')) {
                    if (frame.t === 'snap') baseline = frame.data.items ?? [];
                } else {
                    cursor = frame.cursor;
                    const rows = frame.t === 'snap' ? frame.data.items ?? [] : (frame.changes ?? []).flatMap((change: any) => change.row ? [change.row] : change.data?.items ?? []);
                    for (const row of rows) if (row.collection === 'messages.message' && !baseline.some(item => item.id === row.id)) captures.set(row.id, row);
                    return;
                }
            }
            socket.send(data);
        });
    });
    await page.goto('/__fixtures/commands?chat=' + encodeURIComponent(ids['message-chat']));
    await expect(page.getByRole('button', { name: 'Send fixture message', exact: true })).toBeEnabled();
    await expect.poll(() => baseline.length).toBeGreaterThan(0);
    return {
        sent, statuses, captures,
        diagnose: async () => {
            const id=sent.find(frame => frame.t==='cmd')?.id;
            const response=id ? await request.post(process.env.AB_WEB_CONTROL!+'/command-effect',{headers:{'X-Bridge-Lab':process.env.AB_WEB_CONTROL_KEY!},data:{command_id:id},timeout:5000}).catch(() => undefined) : undefined;
            return {frames:sent.map(frame => ({t:frame.t,id:frame.id,op:frame.op,view:frame.view})),statuses:statuses.map(frame => ({id:frame.id,status:frame.status,state:frame.command?.state,disposition:frame.command?.disposition,parts:frame.command?.parts?.map((part:any) => ({index:part.index,disposition:part.disposition,has_record_id:Boolean(part.record_id)}))})),captures:captures.size,effect:response?.ok() ? await response.json() : undefined,live:await page.getByLabel('Fixture live command count').textContent(),owned:await page.getByLabel('Fixture owned command count').textContent(),echoes:await page.getByLabel('Fixture echo count').textContent()};
        },
        baselineCount:()=>baseline.length,
        command: () => sent.find(frame => frame.t === 'cmd'),
        freeze: () => { freezeStatus = true; },
        thaw: () => { freezeStatus = false; frozen.splice(0).forEach(emit); },
        status: (frame: any) => emit(frame),
        publish: (records: any[], at = cursor) => emit({ t: 'snap', sub: thread, cursor: at, data: { items: [...baseline, ...records] } }),
        ready: async (fault = '', files = true) => {
            if (fault) await control(request, '/message-fault?name=' + fault);
            await page.getByRole('textbox', { name: 'Fixture message', exact: true }).fill('[ab-test compose] multipart contract');
            if (files) {
                await page.getByLabel('Fixture attachments').setInputFiles([
                    { name: 'bridge-part-one.txt', mimeType: 'text/plain', buffer: Buffer.from('first fixture attachment') },
                    { name: 'bridge-part-two.txt', mimeType: 'text/plain', buffer: Buffer.from('second fixture attachment') }
                ]);
                await expect(page.getByLabel('Fixture ready attachments')).toHaveText('2');
                await expect(page.getByRole('alert')).toBeEmpty();
                expect(sent.filter(frame => frame.t === 'cmd')).toHaveLength(0);
            }
        },
        send: () => page.getByRole('button', { name: 'Send fixture message', exact: true }).click()
    };
}
async function finalParts(f: Awaited<ReturnType<typeof fixture>>) {
    try { await expect.poll(() => f.statuses.some(frame => frame.command?.parts?.length === 3 && frame.command.parts.every((part: any) => part.record_id || part.disposition === 'failed_no_effect'))).toBe(true); }
    catch(error) { await test.info().attach('part-command-status-diagnostic',{contentType:'application/json',body:JSON.stringify(await f.diagnose())}); throw error; }
    return f.statuses.findLast(frame => frame.command?.parts?.length === 3 && frame.command.parts.every((part: any) => part.record_id || part.disposition === 'failed_no_effect'));
}
async function effect(request: APIRequestContext, id: string) {
    const response = await request.post(process.env.AB_WEB_CONTROL! + '/command-effect', { headers: { 'X-Bridge-Lab': process.env.AB_WEB_CONTROL_KEY! }, data: { command_id: id } });
    expect(response.ok()).toBe(true);
    expect((await response.json()).count).toBe(1);
}
test('WEB-MSG-PARTS SWAP first-frame parts swap independently only with exact IDs and a compatible cursor', async ({ page, context, request }) => {
    const f = await fixture(page, context, request);
    await f.ready();
    await page.evaluate(() => {
        (window as any).__partFrames = [];
        window.addEventListener('click', event => {
            if ((event.target as Element).closest('[data-compose-send]')) requestAnimationFrame(() => { (window as any).__partFrames.push([...document.querySelectorAll('[data-local-echo]')].map(row => ({ id: row.getAttribute('data-record-id'), index: row.getAttribute('data-command-part-index') }))); });
        }, { capture: true });
    });
    await page.evaluate(() => {
        (window as any).__partCopies = [];
        Object.defineProperty(navigator,'clipboard',{configurable:true,value:{writeText:async (text:string) => { (window as any).__partCopies.push(text); }}});
    });
    await f.send();
    await page.getByRole('button',{name:'Copy fixture text part',exact:true}).click();
    expect(await page.evaluate(() => (window as any).__partCopies)).toEqual(['[ab-test compose] multipart contract']);
    const final = await finalParts(f), id = f.command().id, parts = final.command.parts;
    await expect.poll(() => parts.every((part: any) => f.captures.has(part.record_id))).toBe(true);
    expect(await page.evaluate(() => (window as any).__partFrames[0])).toEqual([0, 1, 2].map(index => ({ id: 'command:' + id + ':part:' + index, index: String(index) })));
    const records = parts.map((part: any) => f.captures.get(part.record_id));
    const clockTime=await page.evaluate(() => Date.now());
    await page.clock.install({time:clockTime}); await page.clock.pauseAt(clockTime+100);
    f.publish([records[2]], { ...final.command.cursor, sequence: final.command.commit_seq - 1 });
    await expect.poll(() => page.getByLabel('Fixture captured sequence').textContent()).toBe(String(final.command.commit_seq-1));
    await page.clock.runFor(34);
    await expect(page.locator('[data-local-echo]')).toHaveCount(3);
    await expect(page.locator('[data-record-id="' + parts[2].record_id + '"]')).toHaveCount(0);
    f.publish([records[2]], final.command.cursor);
    await expect.poll(() => page.getByLabel('Fixture captured sequence').textContent()).toBe(String(final.command.cursor.sequence));
    await page.clock.runFor(34);
    await expect(page.locator('[data-local-echo]')).toHaveCount(2);
    await expect(page.locator('[data-record-id="' + parts[2].record_id + '"][data-command-part-index="2"]')).toHaveCount(1);
    expect(await page.locator('[data-command-id="'+id+'"]').evaluateAll(rows => rows.map(row => Number(row.getAttribute('data-command-part-index'))))).toEqual([0,1,2]);
    f.publish(records, final.command.cursor);
    await expect.poll(() => page.getByLabel('Fixture captured count').textContent()).toBe(String(f.baselineCount()+3));
    await page.clock.runFor(34);
    await expect(page.locator('[data-local-echo]')).toHaveCount(0);
    await page.clock.resume();
    for (const part of parts) {
        const row = page.locator('[data-record-id="' + part.record_id + '"]');
        await expect(row).toHaveCount(1);
        await expect(row).toHaveAttribute('data-command-id', id);
        await expect(row).toHaveAttribute('data-command-part-index', String(part.index));
        await expect(row).toHaveAttribute('data-record-version', String(f.captures.get(part.record_id).version));
    }
    await expect(page.getByLabel('Captured metadata untouched')).toHaveText('true');
    expect(f.sent.filter(frame => frame.t === 'cmd')).toHaveLength(1);
    await effect(request, id);
});
for (const fault of ['part-delivery-failed', 'part-no-effect']) test('WEB-MSG-PARTS ' + (fault === 'part-delivery-failed' ? 'CAPTURED-FAILURE' : 'NO-EFFECT') + ' mixed outcomes preserve successful parts and distinguish ' + fault, async ({ page, context, request }) => {
    const f = await fixture(page, context, request);
    await f.ready(fault); await f.send();
    const final = await finalParts(f), id = f.command().id, parts = final.command.parts;
    const bound = parts.filter((part: any) => part.record_id);
    await expect.poll(() => bound.every((part: any) => f.captures.has(part.record_id))).toBe(true);
    f.publish(bound.map((part: any) => f.captures.get(part.record_id)), final.command.cursor);
    for (const part of parts.slice(0, 2)) {
        await expect(page.locator('[data-record-id="' + part.record_id + '"][data-command-id="' + id + '"][data-command-part-index="' + part.index + '"]')).toHaveCount(1);
    }
    if (fault === 'part-delivery-failed') {
        expect(parts[2].disposition).toBe('delivery_failed');
        await expect(page.locator('[data-local-echo]')).toHaveCount(0);
        await expect(page.locator('[data-record-id="' + parts[2].record_id + '"]')).toHaveAttribute('data-send-state', 'failed');
    } else {
        expect(parts[2].disposition).toBe('failed_no_effect');
        expect(parts[2].record_id).toBeUndefined();
        await expect(page.locator('[data-local-echo]')).toHaveCount(1);
        await expect(page.locator('[data-local-echo][data-command-part-index="2"]')).toHaveAttribute('data-send-state', 'failed');
    }
    await expect(page.getByLabel('Fixture command status')).toHaveText('Not Delivered');
    expect(f.sent.filter(frame => frame.t === 'cmd')).toHaveLength(1);
    await effect(request, id);
});
test('WEB-MSG-PARTS STALE stale per-part evidence cannot erase a verified mapping or regress its state', async ({ page, context, request }) => {
    const f = await fixture(page, context, request);
    await f.ready(); await f.send();
    const final = await finalParts(f), id = f.command().id, parts = final.command.parts;
    await expect.poll(() => parts.every((part: any) => f.captures.has(part.record_id))).toBe(true);
    f.publish(parts.map((part: any) => f.captures.get(part.record_id)), final.command.cursor);
    await expect(page.locator('[data-command-id="' + id + '"][data-command-part-index]')).toHaveCount(3);
    const stale = structuredClone(final);
    stale.command.parts = parts.map((part: any) => ({ ...part, record_id: undefined, disposition: 'indeterminate' }));
    f.status(stale);
    await page.evaluate(() => new Promise(requestAnimationFrame));
    await expect(page.locator('[data-local-echo]')).toHaveCount(0);
    for (const part of parts) await expect(page.locator('[data-record-id="' + part.record_id + '"]')).toHaveAttribute('data-command-part-index', String(part.index));
    await expect(page.getByLabel('Fixture command status')).toHaveText('Sent');
    expect(f.sent.filter(frame => frame.t === 'cmd')).toHaveLength(1);
});
for (const candidate of [false, true]) test('WEB-MSG-PARTS GHOST-' + (candidate ? 'CANDIDATE' : 'ECHO') + ' unresolved delivery failure ' + (candidate ? 'uses a notice after releasing a native candidate' : 'retains its echo without inventing an identity'), async ({ page, context, request }) => {
    const f = await fixture(page, context, request);
    await f.ready('ghost-send', false); f.freeze(); await f.send();
    await expect.poll(() => f.statuses.some(frame => frame.command?.disposition === 'delivery_failed')).toBe(true);
    const final = f.statuses.findLast(frame => frame.command?.disposition === 'delivery_failed');
    expect(final.command.parts[0].record_id).toBeUndefined();
    if (candidate) {
        await control(request, '/incoming');
        await expect.poll(() => [...f.captures.values()].some(row => row.data.is_from_me && row.data.chat_id === f.command().args.chat_id)).toBe(true);
        f.publish([...f.captures.values()]);
        await expect(page.getByLabel('Projection timeout count')).toHaveText('1');
        await expect(page.locator('[data-local-echo]')).toHaveCount(0);
    }
    f.thaw();
    await expect(page.getByLabel('Fixture command status')).toHaveText('Not Delivered');
    await expect(page.getByLabel('Fixture retry allowed')).toHaveText('false');
    if (candidate) {
        await expect(page.getByLabel('Uncertain delivery')).toContainText("1 message wasn't delivered");
        await expect(page.locator('[data-local-echo]')).toHaveCount(0);
    } else {
        await expect(page.locator('[data-local-echo]')).toHaveCount(1);
        await expect(page.locator('[data-local-echo]')).toHaveAttribute('data-send-state', 'failed');
        await expect(page.getByLabel('Uncertain delivery')).toHaveCount(0);
    }
    expect(f.sent.filter(frame => frame.t === 'cmd')).toHaveLength(1);
    expect(f.sent.filter(frame => frame.t === 'resolve')).toHaveLength(0);
});
test('WEB-MSG-PARTS STRENGTHEN Check strengthens one unknown part while another stays unknown and a no-effect failure remains visible', async ({ page, context, request }) => {
    const f = await fixture(page, context, request);
    await f.ready('part-progress-failure'); await f.send();
    await expect.poll(() => f.statuses.some(frame => frame.command?.parts?.length === 3 && frame.command.parts[2].disposition === 'failed_no_effect')).toBe(true);
    const initial = f.statuses.findLast(frame => frame.command?.parts?.length === 3);
    expect(initial.command.parts.map((part: any) => part.disposition)).toEqual(['indeterminate', 'indeterminate', 'failed_no_effect']);
    expect(initial.command.parts.every((part: any) => !part.record_id)).toBe(true);
    await expect.poll(() => [...f.captures.values()].filter(row => row.data.is_from_me).length).toBe(2);
    f.publish([...f.captures.values()]);
    const notice = page.getByLabel('Uncertain delivery');
    await expect(notice).toContainText('2 messages may not have been delivered');
    await expect(page.locator('[data-local-echo]')).toHaveCount(1);
    await expect(page.locator('[data-local-echo][data-command-part-index="2"]')).toHaveAttribute('data-send-state', 'failed');
    await page.getByRole('button', { name: 'Check fixture command', exact: true }).click();
    await expect.poll(() => f.statuses.some(frame => frame.command?.parts?.[0]?.record_id && frame.command.parts[1].disposition === 'indeterminate')).toBe(true);
    const strengthened = f.statuses.findLast(frame => frame.command?.parts?.[0]?.record_id);
    const first = strengthened.command.parts[0];
    expect(f.captures.has(first.record_id)).toBe(true);
    f.publish([...f.captures.values()], strengthened.command.cursor);
    await expect(page.locator('[data-record-id="' + first.record_id + '"]')).toHaveAttribute('data-command-part-index', '0');
    await expect(notice).toContainText('1 message may not have been delivered');
    await expect(page.locator('[data-local-echo]')).toHaveCount(1);
    await expect(page.locator('[data-local-echo][data-command-part-index="2"]')).toHaveAttribute('data-send-state', 'failed');
    expect(f.sent.filter(frame => frame.t === 'cmd')).toHaveLength(1);
    expect(f.sent.filter(frame => frame.t === 'resolve').map(frame => frame.id)).toEqual([f.command().id]);
    await effect(request, f.command().id);
});

for(const fault of ['part-delivery-failed','part-no-effect']) test('WEB-MSG-PARTS RETRY '+fault+' retries only the explicit failed part with a new input and upload',async ({page,context,request}) => {
    const f=await fixture(page,context,request);
    await f.ready(fault); await f.send();
    const final=await finalParts(f), original=f.command(), failed=final.command.parts[2];
    const bound=final.command.parts.filter((part:any) => part.record_id);
    await expect.poll(() => bound.every((part:any) => f.captures.has(part.record_id))).toBe(true);
    f.publish(bound.map((part:any) => f.captures.get(part.record_id)),final.command.cursor);
    await control(request,'/message-fault?name=');
    await page.getByRole('button',{name:'Prepare fixture failed part',exact:true}).click();
    await expect(page.getByLabel('Retry upload ready')).toHaveText('true');
    expect(f.sent.filter(frame => frame.t==='cmd')).toHaveLength(1);
    const retry=page.getByRole('button',{name:'Retry fixture failed part',exact:true});
    await retry.evaluate(button => button.dispatchEvent(new MouseEvent('click',{bubbles:true})));
    await page.evaluate(() => new Promise(requestAnimationFrame));
    expect(f.sent.filter(frame => frame.t==='cmd')).toHaveLength(1);
    await retry.click();
    await expect.poll(() => f.sent.filter(frame => frame.t==='cmd').length).toBe(2);
    const next=f.sent.filter(frame => frame.t==='cmd')[1];
    expect(next.id).not.toBe(original.id); expect(next.args.chat_id).toBe(original.args.chat_id);
    expect(next.args.text).toBeUndefined(); expect(next.args.files).toHaveLength(1);
    expect(next.args.files[0].name).toBe(original.args.files[1].name);
    expect(next.args.files[0].upload_id).not.toBe(original.args.files[1].upload_id);
    expect(next.args.resend_of).toBe(failed.record_id);
    await expect.poll(() => f.statuses.some(frame => frame.id===next.id && frame.command?.state==='observed')).toBe(true);
    await retry.click(); await page.evaluate(() => new Promise(requestAnimationFrame));
    expect(f.sent.filter(frame => frame.t==='cmd')).toHaveLength(2);
    for(const part of final.command.parts.slice(0,2))await expect(page.locator('[data-record-id="'+part.record_id+'"][data-command-id="'+original.id+'"]')).toHaveCount(1);
    await effect(request,original.id); await effect(request,next.id);
});

test('WEB-MSG-PARTS LATE-FAILURE an exact released failed row does not gain a duplicate echo while its cursor catches up',async ({page,context,request}) => {
    const f=await fixture(page,context,request);
    await f.ready('part-delivery-failed'); f.freeze(); await f.send();
    const final=await finalParts(f), failed=final.command.parts[2];
    await expect.poll(() => final.command.parts.every((part:any) => f.captures.has(part.record_id))).toBe(true);
    f.publish([f.captures.get(failed.record_id)],{...final.command.cursor,sequence:final.command.commit_seq-1});
    await expect(page.getByLabel('Projection timeout count')).toHaveText('1');
    await expect(page.locator('[data-local-echo]')).toHaveCount(0);
    f.thaw();
    await expect(page.getByLabel('Fixture command status')).toHaveText('Not Delivered');
    await expect(page.locator('[data-local-echo][data-command-part-index="2"]')).toHaveCount(0);
    await expect(page.locator('[data-record-id="'+failed.record_id+'"]')).toHaveCount(1);
    f.publish(final.command.parts.map((part:any) => f.captures.get(part.record_id)),final.command.cursor);
    await expect(page.locator('[data-local-echo]')).toHaveCount(0);
    await expect(page.locator('[data-record-id="'+failed.record_id+'"][data-command-part-index="2"]')).toHaveAttribute('data-send-state','failed');
    expect(f.sent.filter(frame => frame.t==='cmd')).toHaveLength(1);
    await effect(request,f.command().id);
});

test('WEB-MSG-PARTS PENDING local part status is reactive before any command acknowledgement',async ({page,context,request}) => {
    const f=await fixture(page,context,request);
    await f.ready(); f.freeze(); await f.send();
    await expect(page.getByLabel('Fixture command status')).toHaveText('Sending…');
    await expect(page.getByLabel('Fixture echo count')).toHaveText('3');
    const final=await finalParts(f);
    await expect(page.getByLabel('Fixture command status')).toHaveText('Sending…');
    await expect.poll(() => final.command.parts.every((part:any) => f.captures.has(part.record_id))).toBe(true);
    f.thaw(); f.publish(final.command.parts.map((part:any) => f.captures.get(part.record_id)),final.command.cursor);
    await expect(page.getByLabel('Fixture command status')).toHaveText('Sent');
    await expect(page.locator('[data-local-echo]')).toHaveCount(0);
    expect(f.sent.filter(frame => frame.t==='cmd')).toHaveLength(1);
    await effect(request,f.command().id);
});
