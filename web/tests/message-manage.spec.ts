import { test, expect, type BrowserContext, type Page } from '@playwright/test';
import { control } from './session';
import {readFile} from 'node:fs/promises';
import {createHash} from 'node:crypto';
let cookies: Parameters<BrowserContext['addCookies']>[0] = [], chat = '';
test.beforeAll(async ({request}) => { cookies = [(await control(request,'/session')).cookie]; chat = (await control(request,'/demo-fixtures'))['message-chat']; });
async function fixture(page:Page,context:BrowserContext,holdProof=false,mutate?:(record:any)=>void) {
    await page.goto(new URL(cookies[0].url!).origin);
    await expect(page.getByRole('heading',{name:'Sign in with a passkey',exact:true})).toBeVisible();
    await context.addCookies(cookies);
    const errors:any[]=[],commands:any[] = [], statuses:any[] = [], resolves:string[] = [], proofs:(()=>void)[] = [];
    const commandViews=new Map<string,string>(); let holdResults=false,releaseOnResolve=false, close=()=>{};
    await page.routeWebSocket('**/web/v1/live',socket => {
        const server = socket.connectToServer(); close=()=>{void socket.close({code:4000,reason:'fixture_transport_loss'});};
        socket.onMessage(data => { const frame = JSON.parse(String(data)); if(frame.t === 'cmd')commands.push(frame); if(frame.t==='resolve'){resolves.push(frame.id);if(releaseOnResolve)holdResults=false;} if(frame.t==='sub' && frame.args?.collection==='system.command')commandViews.set(frame.id,frame.args.id); server.send(data); });
        server.onMessage(data => {
            const frame = JSON.parse(String(data));
            if(frame.t === 'cmd_status')statuses.push(frame);if(frame.t==='error')errors.push(frame);
            const resultID=frame.t==='cmd_status' ? frame.id : commandViews.get(frame.sub);
            if(holdResults && commands.some(command => ['messages.delete_message'].includes(command.op) && command.id===resultID))return;
            if(holdProof && frame.data?.records?.some((record:any) => record.collection === 'system.source' && record.source === 'messages')) {
                proofs.push(() => socket.send(data));
                for(const record of frame.data.records)if(record.collection === 'system.source' && record.source === 'messages') {
                    record.data.capabilities = record.data.capabilities?.filter((operation:string) => !['messages.delete_message'].includes(operation));
                    for(const proof of record.data.coverage?.operations??[])if(['messages.delete_message'].includes(proof.operation)){proof.proven=false;proof.reason='Waiting for verified change capability';}
                }
                socket.send(JSON.stringify(frame)); return;
            }
            if(mutate){const visit=(value:any)=>{if(!value||typeof value!=='object')return;if(value.data&&typeof value.data==='object'&&(value.collection==='messages.message'||value.collection==='messages.chat'))mutate(value);for(const child of Object.values(value))if(child&&typeof child==='object')Array.isArray(child)?child.forEach(visit):visit(child);};visit(frame);socket.send(JSON.stringify(frame));}else socket.send(data);
        });
    });
    const sessionResponse=page.waitForResponse(response=>new URL(response.url()).pathname==='/web/v1/session');
    await page.goto('/__fixtures/message-actions?chat='+encodeURIComponent(chat));
    const sessionRead=await sessionResponse;
    if(sessionRead.status()===429){const seconds=Number(sessionRead.headers()['retry-after']);expect(seconds).toBeGreaterThan(0);expect(seconds).toBeLessThanOrEqual(2);test.info().annotations.push({type:'fixture',description:'Honored session429 Retry-After through the actual Try again button'});await page.waitForTimeout(seconds*1000);const retried=page.waitForResponse(response=>new URL(response.url()).pathname==='/web/v1/session');await page.getByRole('button',{name:'Try again',exact:true}).click();expect((await retried).ok()).toBe(true);}

    await page.getByRole('button',{name:'Create self fixture message',exact:true}).click();
    const id = page.getByLabel('Created fixture message');
    await expect(id).not.toBeEmpty();
    const recordID = await id.textContent(), target = page.locator('[data-message="'+recordID+'"]');
    await expect(target).toBeVisible();
    return {errors,commands,statuses,resolves,recordID,target,close:()=>close(),holdResults:()=>{holdResults=true;},releaseResults:()=>{holdResults=false;},releaseOnResolve:()=>{releaseOnResolve=true;},releaseProof:() => {holdProof=false;proofs.splice(0).forEach(release => release());}};
}
async function openMain(page:Page,f:Awaited<ReturnType<typeof fixture>>){
    await page.goto('/messages/'+encodeURIComponent(chat)+'/'+encodeURIComponent(f.recordID!));
    const row=page.locator('.message-row[data-message="'+f.recordID+'"]');await expect(row).toBeVisible();await row.locator('.bubble[data-part="0"]').click({button:'right'});
    return {row,menu:page.getByRole('dialog',{name:'Message actions',exact:true})};
}
test('WEB-MSG-MANAGE Delete waits for a new trusted confirmation and uses its captured version',async({page,context})=>{
    const f=await fixture(page,context),{row,menu}=await openMain(page,f),version=Number(await menu.getAttribute('data-message-version'));
    await expect(menu.getByRole('button',{name:'Delete',exact:true})).toBeVisible();await menu.getByRole('button',{name:'Delete',exact:true}).click();
    const confirm=page.getByRole('dialog',{name:'Delete Message',exact:true});await expect(confirm).toBeVisible();expect(f.commands.filter(command=>command.op==='messages.delete_message')).toHaveLength(0);
    await confirm.getByRole('button',{name:'Cancel',exact:true}).click();await expect(confirm).toHaveCount(0);await expect(row).toBeVisible();
    await row.locator('.bubble[data-part="0"]').click({button:'right'});await menu.getByRole('button',{name:'Delete',exact:true}).click();
    await confirm.getByRole('button',{name:'Delete',exact:true}).dispatchEvent('click');expect(f.commands.filter(command=>command.op==='messages.delete_message')).toHaveLength(0);
    await confirm.getByRole('button',{name:'Delete',exact:true}).click();await expect.poll(()=>f.commands.filter(command=>command.op==='messages.delete_message').length).toBe(1);
    const command=f.commands.find(command=>command.op==='messages.delete_message');expect(command.args).toEqual({id:f.recordID});expect(command.expected_version).toBe(version);await expect(confirm).toHaveCount(0);await expect.poll(()=>f.statuses.findLast(frame=>frame.id===command.id)?.command?.state).toBe('observed');await expect(row).toHaveCount(0);
});
test('WEB-MSG-MANAGE Forward pre-fills a separate new message without sending or overwriting its draft',async({page,context})=>{
    const f=await fixture(page,context);await page.goto('/messages/new');const field=page.getByRole('textbox',{name:'Message',exact:true});await field.fill('[ab-test actions] existing new draft');
    const {menu}=await openMain(page,f);await expect(menu.getByRole('button',{name:'Forward',exact:true})).toBeVisible();await menu.getByRole('button',{name:'Forward',exact:true}).click();
    await expect(page).toHaveURL(/\/messages\/new(?:\?|$)/u);await expect(page.getByRole('textbox',{name:'Message',exact:true})).toHaveValue('[ab-test actions] fresh self message');
    expect(f.commands.filter(command=>command.op==='messages.send')).toHaveLength(1);expect(await page.evaluate(()=>{window.dispatchEvent(new Event('pagehide'));return Object.keys(localStorage).filter(key=>key.startsWith('ab.messages.draft.')&&key.includes('forward%3A'));})).toEqual([]);
    await page.goto('/messages/'+encodeURIComponent(chat));await page.goto('/messages/new');await expect(page.getByRole('textbox',{name:'Message',exact:true})).toHaveValue('[ab-test actions] existing new draft');
});

test('WEB-MSG-MANAGE denied Copy keeps the captured message available and dispatches no write',async({page,context})=>{
    await page.addInitScript(()=>Object.defineProperty(navigator,'clipboard',{configurable:true,value:{writeText:async()=>{throw new DOMException('Clipboard unavailable','NotAllowedError');}}}));
    const f=await fixture(page,context),{menu,row}=await openMain(page,f);await menu.getByRole('button',{name:'Copy',exact:true}).dispatchEvent('click');await expect(menu.getByRole('status')).toHaveCount(0);
    await menu.getByRole('button',{name:'Copy',exact:true}).click();await expect(menu.getByRole('status')).toHaveText("Couldn't copy this message.");await expect(menu).toBeVisible();await menu.getByRole('button',{name:'Close',exact:true}).click();await expect(row).toContainText('[ab-test actions] fresh self message');expect(f.commands.filter(command=>command.op!=='messages.send')).toHaveLength(0);
});
test('WEB-MSG-MANAGE Save Attachment downloads its exact captured original without a write',async({page,context,request})=>{
    const f=await fixture(page,context),ids=await control(request,'/demo-fixtures'),id=ids['demo-message-00-000040'];await page.goto('/messages/'+encodeURIComponent(chat)+'/'+encodeURIComponent(id));
    const row=page.locator('[data-message="'+id+'"]'),chip=row.locator('[data-file-mime="application/pdf"]');await expect(chip).toBeVisible();const digest=await chip.getAttribute('data-file-digest');
    await row.locator('.bubble[data-part="0"]').click({button:'right'});const menu=page.getByRole('dialog',{name:'Message actions',exact:true});const save=menu.getByRole('button',{name:'Save Attachment',exact:true});await expect(save).toBeVisible();
    const downloaded=page.waitForEvent('download');await save.click();const download=await downloaded;expect(download.suggestedFilename()).toBe('demo-field-notes.pdf');expect(createHash('sha256').update(await readFile((await download.path())!)).digest('hex')).toBe(digest);expect(f.commands.filter(command=>command.op!=='messages.send')).toHaveLength(0);
});
test('WEB-MSG-MANAGE an unproven Delete stays hidden until its fresh operation proof arrives',async({page,context})=>{
    const f=await fixture(page,context,true),{menu}=await openMain(page,f);await expect(menu.getByRole('button',{name:'Delete',exact:true})).toHaveCount(0);f.releaseProof();await expect(menu.getByRole('button',{name:'Delete',exact:true})).toBeVisible();expect(f.commands.filter(command=>command.op==='messages.delete_message')).toHaveLength(0);
});
test('WEB-MSG-MANAGE WEB-MSG-MANAGE-CHECK a lost Delete result is reconciled by the original ID and effect remains one',async({page,context,request})=>{
    const f=await fixture(page,context),{menu}=await openMain(page,f);await menu.getByRole('button',{name:'Delete',exact:true}).click();f.holdResults();await page.getByRole('dialog',{name:'Delete Message',exact:true}).getByRole('button',{name:'Delete',exact:true}).click();
    await expect.poll(()=>f.statuses.some(frame=>frame.id===f.commands.find(command=>command.op==='messages.delete_message')?.id&&['observed','delivered'].includes(frame.command?.state??frame.status))).toBe(true);const original=f.commands.find(command=>command.op==='messages.delete_message');f.close();
    const notice=page.getByRole('status',{name:'Uncertain delivery',exact:true});await expect(notice).toContainText('Not sure if the message was deleted');f.releaseOnResolve();await expect(notice.getByRole('button',{name:'Check',exact:true})).toBeEnabled();const beforeCheck=f.statuses.length;await notice.getByRole('button',{name:'Check',exact:true}).click();await expect.poll(()=>f.resolves).toEqual([original.id]);await expect.poll(()=>{const frame=f.statuses.slice(beforeCheck).findLast(frame=>frame.id===original.id);return frame?.command?.state??frame?.status??f.errors.findLast(frame=>frame.id===original.id)?.code??'waiting for command status';},{timeout:16000,message:'Resolve returns the original command within its 15 second server bound'}).toMatch(/^(observed|delivered)$/u);await expect(page.locator('.messages-transcript')).toHaveAttribute('aria-busy','false',{timeout:30000});await expect(notice).toHaveCount(0);
    expect(f.resolves).toEqual([original.id]);expect(f.commands.filter(command=>command.op==='messages.delete_message').map(command=>command.id)).toEqual([original.id]);const effect=await request.post(process.env.AB_WEB_CONTROL!+'/command-effect',{headers:{'X-Bridge-Lab':process.env.AB_WEB_CONTROL_KEY!},data:{command_id:original.id}});expect(effect.ok()).toBe(true);expect((await effect.json()).count).toBe(1);
});

test('WEB-MSG-MANAGE a stale captured version rejects Delete without hiding the row or retrying',async({page,context,request})=>{
    const f=await fixture(page,context,false,record=>{if(record.collection==='messages.message')record.version+=1;}),{row,menu}=await openMain(page,f);
    await menu.getByRole('button',{name:'Delete',exact:true}).click();await page.getByRole('dialog',{name:'Delete Message',exact:true}).getByRole('button',{name:'Delete',exact:true}).click();
    await expect.poll(()=>f.statuses.some(frame=>frame.id===f.commands.find(command=>command.op==='messages.delete_message')?.id&&frame.command?.state==='rejected')).toBe(true);
    await expect(row).toBeVisible();await expect(page.getByRole('status',{name:'Uncertain delivery',exact:true})).toContainText("Message wasn't deleted");
    const writes=f.commands.filter(command=>command.op==='messages.delete_message');expect(writes).toHaveLength(1);const effect=await request.post(process.env.AB_WEB_CONTROL!+'/command-effect',{headers:{'X-Bridge-Lab':process.env.AB_WEB_CONTROL_KEY!},data:{command_id:writes[0].id}});expect(effect.ok()).toBe(true);expect((await effect.json()).count).toBe(0);
});

test('WEB-MSG-MANAGE missing native attachment identity never falls back to filename or order',async({page,context,request})=>{
    const f=await fixture(page,context,false,record=>{if(record.collection==='messages.message'&&Array.isArray(record.data.parts))for(const part of record.data.parts)if(part.kind==='attachment')delete part.attachment_id;}),ids=await control(request,'/demo-fixtures'),id=ids['demo-message-00-000040'];
    await page.goto('/messages/'+encodeURIComponent(chat)+'/'+encodeURIComponent(id));const row=page.locator('[data-message="'+id+'"]');await expect(row).toBeVisible();await row.locator('.bubble[data-part="0"]').click({button:'right'});const menu=page.getByRole('dialog',{name:'Message actions',exact:true});await expect(menu).toBeVisible();await expect(menu.getByRole('button',{name:'Save Attachment',exact:true})).toHaveCount(0);await expect(menu.getByRole('button',{name:'Forward',exact:true})).toHaveCount(0);expect(f.commands.filter(command=>command.op!=='messages.send')).toHaveLength(0);
});
test('WEB-MSG-MANAGE Forward attachment stages the exact captured original and waits for a recipient and Send',async({page,context,request})=>{
    const f=await fixture(page,context),ids=await control(request,'/demo-fixtures'),id=ids['demo-message-00-000040'];await page.goto('/messages/'+encodeURIComponent(chat)+'/'+encodeURIComponent(id));const row=page.locator('[data-message="'+id+'"]');await expect(row.locator('[data-file-mime="application/pdf"]')).toBeVisible();await row.locator('.bubble[data-part="0"]').click({button:'right'});await page.getByRole('dialog',{name:'Message actions',exact:true}).getByRole('button',{name:'Forward',exact:true}).click();await expect(page).toHaveURL(/\/messages\/new$/u);await expect(page.locator('.upload-files [data-upload-state="ready"]')).toHaveCount(1);await expect(page.locator('.upload-files')).toContainText('demo-field-notes.pdf');await expect(page.getByRole('textbox',{name:'Message',exact:true})).toHaveValue('');await expect(page.getByRole('button',{name:'Send message',exact:true})).toBeDisabled();expect(f.commands.filter(command=>command.op==='messages.send')).toHaveLength(1);
});
