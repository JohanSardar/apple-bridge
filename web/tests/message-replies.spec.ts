import { test, expect, type BrowserContext, type Page } from '@playwright/test';
import { control } from './session';
let cookies: Parameters<BrowserContext['addCookies']>[0] = [], chat = '';
test.beforeAll(async ({request}) => { cookies = [(await control(request,'/session')).cookie]; chat = (await control(request,'/demo-fixtures'))['message-chat']; });
async function fixture(page:Page,context:BrowserContext,holdProof=false) {
    await page.goto(new URL(cookies[0].url!).origin);
    await expect(page.getByRole('heading',{name:'Sign in with a passkey',exact:true})).toBeVisible();
    await context.addCookies(cookies);
    const commands:any[] = [], statuses:any[] = [], resolves:string[] = [], proofs:(()=>void)[] = [];
    const commandViews=new Map<string,string>(),captured=new Map<string,any>(),latestViews=new Set<string>();let latestOverride=''; let holdResults=false,releaseOnResolve=false, close=()=>{};
    await page.routeWebSocket('**/web/v1/live',socket => {
        const server = socket.connectToServer(); close=()=>{void socket.close({code:4000,reason:'fixture_transport_loss'});};
        socket.onMessage(data => { const frame = JSON.parse(String(data)); if(frame.t === 'cmd')commands.push(frame); if(frame.t==='resolve'){resolves.push(frame.id);if(releaseOnResolve)holdResults=false;} if(frame.t==='sub' && frame.args?.collection==='system.command')commandViews.set(frame.id,frame.args.id); if(frame.t==='sub'&&frame.view==='thread'&&frame.args?.anchor==='latest'&&frame.args.limit===1)latestViews.add(frame.id);server.send(data); });
        server.onMessage(data => {
            const frame = JSON.parse(String(data));
            for(const item of frame.data?.items??[])captured.set(item.id,item);
            if(frame.t==='snap'&&latestOverride&&latestViews.has(frame.sub)){frame.data.items=[captured.get(latestOverride)];socket.send(JSON.stringify(frame));return;}
            if(frame.t === 'cmd_status')statuses.push(frame);
            const resultID=frame.t==='cmd_status' ? frame.id : commandViews.get(frame.sub);
            if(holdResults && commands.some(command => ['messages.reply','messages.send'].includes(command.op) && command.id===resultID))return;
            if(holdProof && frame.data?.records?.some((record:any) => record.collection === 'system.source' && record.source === 'messages')) {
                proofs.push(() => socket.send(data));
                for(const record of frame.data.records)if(record.collection === 'system.source' && record.source === 'messages') {
                    record.data.capabilities = record.data.capabilities?.filter((operation:string) => operation !== 'messages.reply');
                    const proof = record.data.coverage?.operations?.find((entry:any) => entry.operation === 'messages.reply');
                    if(proof) { proof.proven=false; proof.reason='Waiting for verified reaction capability'; }
                }
                socket.send(JSON.stringify(frame)); return;
            }
            socket.send(data);
        });
    });
    await page.goto('/__fixtures/message-actions?chat='+encodeURIComponent(chat));
    await page.getByRole('button',{name:'Create self fixture message',exact:true}).click();
    const id = page.getByLabel('Created fixture message');
    await expect(id).not.toBeEmpty();
    const recordID = await id.textContent(), target = page.locator('[data-message="'+recordID+'"]');
    await expect(target).toBeVisible();
    return {commands,statuses,resolves,recordID,target,useLatest:(id:string)=>{if(!captured.has(id))throw new Error('Missing captured fixture item');latestOverride=id;},close:()=>close(),holdResults:()=>{holdResults=true;},releaseResults:()=>{holdResults=false;},releaseOnResolve:()=>{releaseOnResolve=true;},releaseProof:() => {holdProof=false;proofs.splice(0).forEach(release => release());}};
}
test('WEB-MSG-REPLY existing-chat composer quotes the exact native part and sends once',async ({page,context}) => {
    const f=await fixture(page,context);
    await page.goto('/messages/'+encodeURIComponent(chat)+'/'+encodeURIComponent(f.recordID!));
    const row=page.locator('.message-row[data-message="'+f.recordID+'"]');
    await expect(row).toBeVisible();
    const composer=page.getByRole('region',{name:'Message composer',exact:true});
    await expect(composer).toBeVisible();
    await row.locator('.bubble[data-part="0"]').click({button:'right'});
    const menu=page.getByRole('dialog',{name:'Message actions',exact:true});
    await menu.getByRole('button',{name:'Reply',exact:true}).click();
    const quote=composer.getByRole('region',{name:'Replying to message',exact:true});
    await expect(quote).toContainText('[ab-test actions] fresh self message');
    expect(f.commands.filter(command=>command.op==='messages.reply')).toHaveLength(0);
    const theme=await page.evaluate(()=>document.documentElement.dataset.theme);
    await page.screenshot({path:'../lab/evidence/private/web/compose-reply-'+test.info().project.name+'-light.png'});
    await page.evaluate(()=>{document.documentElement.dataset.theme='dark';});
    await page.screenshot({path:'../lab/evidence/private/web/compose-reply-'+test.info().project.name+'-dark.png'});
    await page.evaluate(theme=>{if(theme)document.documentElement.dataset.theme=theme;else delete document.documentElement.dataset.theme;},theme);
    await composer.getByRole('textbox',{name:'Message',exact:true}).fill('[ab-test actions] exact-part reply');
    await composer.getByRole('button',{name:'Send message',exact:true}).evaluate(button=>button.dispatchEvent(new MouseEvent('click',{bubbles:true})));
    await page.evaluate(()=>new Promise(requestAnimationFrame));
    expect(f.commands.filter(command=>command.op==='messages.reply')).toHaveLength(0);
    await composer.getByRole('textbox',{name:'Message',exact:true}).press('Enter');
    await expect.poll(()=>f.commands.filter(command=>command.op==='messages.reply').length).toBe(1);
    const command=f.commands.find(command=>command.op==='messages.reply');
    expect(command.args).toEqual({id:f.recordID,part_index:0,text:'[ab-test actions] exact-part reply'});
    await expect(quote).toHaveCount(0);
    await expect.poll(()=>f.statuses.some(frame=>frame.id===command.id&&['observed','delivered'].includes(frame.status))).toBe(true);
    await expect.poll(()=>page.locator('.message-row[data-command-id="'+command.id+'"]').count()).toBe(1);
    await page.keyboard.press('Enter');
    expect(f.commands.filter(command=>command.op==='messages.reply')).toHaveLength(1);
});

test('WEB-MSG-REPLY CANCEL cancel keeps the per-chat draft and never dispatches',async ({page,context}) => {
    const f=await fixture(page,context);
    await page.goto('/messages/'+encodeURIComponent(chat)+'/'+encodeURIComponent(f.recordID!));
    const composer=page.getByRole('region',{name:'Message composer',exact:true});
    await composer.getByRole('textbox',{name:'Message',exact:true}).fill('[ab-test actions] retained draft');
    await page.locator('.message-row[data-message="'+f.recordID+'"] .bubble[data-part="0"]').click({button:'right'});
    await page.getByRole('dialog',{name:'Message actions',exact:true}).getByRole('button',{name:'Reply',exact:true}).click();
    await composer.getByRole('textbox',{name:'Message',exact:true}).evaluate(field=>field.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',isComposing:true,bubbles:true})));
    await expect(composer.getByRole('region',{name:'Replying to message',exact:true})).toBeVisible();
    await composer.getByRole('button',{name:'Cancel reply',exact:true}).click();
    await expect(composer.getByRole('region',{name:'Replying to message',exact:true})).toHaveCount(0);
    await expect(composer.getByRole('textbox',{name:'Message',exact:true})).toHaveValue('[ab-test actions] retained draft');
    expect(f.commands.filter(command=>command.op==='messages.reply')).toHaveLength(0);
});

test('WEB-MSG-REPLY PROOF reply stays hidden until the fresh live capability is proven',async ({page,context}) => {
    const f=await fixture(page,context,true);
    await page.goto('/messages/'+encodeURIComponent(chat)+'/'+encodeURIComponent(f.recordID!));
    await page.locator('.message-row[data-message="'+f.recordID+'"] .bubble[data-part="0"]').click({button:'right'});
    const menu=page.getByRole('dialog',{name:'Message actions',exact:true});
    await expect(menu).toBeVisible();
    await expect(menu.getByRole('button',{name:'Reply',exact:true})).toHaveCount(0);
    f.releaseProof();
    await expect(menu.getByRole('button',{name:'Reply',exact:true})).toBeVisible();
    expect(f.commands.filter(command=>command.op==='messages.reply')).toHaveLength(0);
});
test('WEB-MSG-REPLY SEND existing-chat Send has one local echo in the first resulting frame',async ({page,context}) => {
    const f=await fixture(page,context);
    await page.goto('/messages/'+encodeURIComponent(chat)+'/'+encodeURIComponent(f.recordID!));
    const composer=page.getByRole('region',{name:'Message composer',exact:true});
    await composer.getByRole('textbox',{name:'Message',exact:true}).fill('[ab-test actions] first-frame existing chat');
    f.holdResults();
    await page.evaluate(()=>{
        window.addEventListener('click',event=>{
            if(!(event.target instanceof Element)||!event.target.closest('[aria-label="Send message"]'))return;
            requestAnimationFrame(()=>{(window as any).__replyFirstFrame=[...document.querySelectorAll('.message-row[data-command-id]')].map(row=>row.getAttribute('data-command-id'));});
        },{capture:true,once:true});
    });
    await composer.getByRole('button',{name:'Send message',exact:true}).click();
    await expect.poll(()=>f.commands.filter(command=>command.op==='messages.send').length).toBe(2);
    const command=f.commands.filter(command=>command.op==='messages.send')[1];
    await expect.poll(()=>page.evaluate(()=>(window as any).__replyFirstFrame)).toContain(command.id);
    await expect(page.locator('.message-row[data-command-id="'+command.id+'"]')).toHaveCount(1);
    f.releaseResults();f.close();
    await expect(page.locator('.message-row[data-command-id="'+command.id+'"]')).toHaveCount(1);
    expect(f.commands.filter(value=>value.op==='messages.send').map(value=>value.id)).toEqual([f.commands[0].id,command.id]);
});

test('WEB-MSG-REPLY AMBIGUITY Check reconciles the original reply without a second write',async ({page,context,request}) => {
    const f=await fixture(page,context);
    await page.goto('/messages/'+encodeURIComponent(chat)+'/'+encodeURIComponent(f.recordID!));
    await page.locator('.message-row[data-message="'+f.recordID+'"] .bubble[data-part="0"]').click({button:'right'});
    await page.getByRole('dialog',{name:'Message actions',exact:true}).getByRole('button',{name:'Reply',exact:true}).click();
    const composer=page.getByRole('region',{name:'Message composer',exact:true});
    await composer.getByRole('textbox',{name:'Message',exact:true}).fill('[ab-test actions] lost reply result');
    f.holdResults();
    await composer.getByRole('button',{name:'Send message',exact:true}).click();
    await expect.poll(()=>f.statuses.some(frame=>frame.id===f.commands.find(command=>command.op==='messages.reply')?.id&&frame.command?.state==='observed')).toBe(true);
    const original=f.commands.find(command=>command.op==='messages.reply');
    f.close();
    await expect(composer).toContainText('Not sure if delivered');
    f.releaseOnResolve();
    await expect(composer.getByRole('button',{name:'Check',exact:true})).toBeEnabled();
    await composer.getByRole('button',{name:'Check',exact:true}).click();
    await expect(composer).not.toContainText('Not sure if delivered');
    expect(f.resolves).toEqual([original.id]);
    expect(f.commands.filter(command=>command.op==='messages.reply').map(command=>command.id)).toEqual([original.id]);
    const effect=await request.post(process.env.AB_WEB_CONTROL!+'/command-effect',{headers:{'X-Bridge-Lab':process.env.AB_WEB_CONTROL_KEY!},data:{command_id:original.id}});
    expect(effect.ok()).toBe(true);expect((await effect.json()).count).toBe(1);
});

test('WEB-MSG-REPLY CELL an actual grouped native cell anchors keyboard actions and replies to its owning part',async ({page,context,request}) => {
    const f=await fixture(page,context),ids=await control(request,'/demo-fixtures');
    const leading=ids['demo-message-00-000042'],member=ids['demo-message-00-000043'];
    await page.goto('/messages/'+encodeURIComponent(chat)+'/'+encodeURIComponent(leading));
    const stack=page.locator('.media-stack'),cell=stack.locator('[data-message="'+member+'"]');
    await expect(stack).toBeVisible();
    await expect(stack.locator('.media-stack-cell')).toHaveCount(4);
    if(test.info().project.name.endsWith('-phone')){await stack.locator('.media-stack-cell').first().locator('button.media-open').focus();await page.keyboard.press('ArrowRight');await expect(stack).toHaveAttribute('data-stack-index','1');await expect(stack).toHaveAttribute('data-stack-motion','settled');}
    await expect(cell).not.toHaveAttribute('inert','');
    expect(await cell.evaluate(element=>element.closest('[data-part]')?.getAttribute('data-part'))).toBe('0');
    await page.getByRole('textbox',{name:'Message',exact:true}).fill('[ab-test actions] unsent cell draft');
    await expect(page.getByRole('button',{name:'Send message',exact:true})).toBeVisible();
    f.useLatest(member);
    await page.keyboard.press('Meta+t');
    const menu=page.getByRole('dialog',{name:'Message actions',exact:true});
    await expect(menu).toBeVisible();await expect(cell).toHaveClass(/message-action-anchor/u);
    await page.keyboard.press('Escape');
    await cell.locator('button.media-open').click({button:'right'});
    await menu.getByRole('button',{name:'Reply',exact:true}).click();
    const composer=page.getByRole('region',{name:'Message composer',exact:true});
    await expect(composer.getByRole('region',{name:'Replying to message',exact:true})).toContainText('Attachment');
    await composer.getByRole('textbox',{name:'Message',exact:true}).fill('[ab-test actions] exact grouped member reply');
    await composer.getByRole('button',{name:'Send message',exact:true}).click();
    await expect.poll(()=>f.commands.filter(command=>command.op==='messages.reply').length).toBe(1);
    expect(f.commands.find(command=>command.op==='messages.reply').args).toEqual({id:member,part_index:0,text:'[ab-test actions] exact grouped member reply'});
    expect(f.commands.filter(command=>command.op==='messages.react')).toHaveLength(0);
});

test('WEB-MSG-REPLY KEYBOARD Command-R quotes the latest exact captured part without sending until Enter',async ({page,context}) => {
    const f=await fixture(page,context);
    await page.goto('/messages/'+encodeURIComponent(chat)+'/'+encodeURIComponent(f.recordID!));
    const composer=page.getByRole('region',{name:'Message composer',exact:true});
    await composer.getByRole('textbox',{name:'Message',exact:true}).fill('[ab-test actions] keyboard reply');
    await expect(composer.getByRole('button',{name:'Send message',exact:true})).toBeVisible();
    await expect(page.locator('.messages-transcript')).toHaveAttribute('data-window-end',/^msg_/u);
    const latest=await page.locator('.messages-transcript').getAttribute('data-window-end');
    const part=Number(await page.locator('.message-row[data-message="'+latest+'"] .bubble[data-part]').last().getAttribute('data-part'));
    await page.keyboard.press('Meta+r');
    await expect(composer.getByRole('region',{name:'Replying to message',exact:true})).toBeVisible();
    expect(f.commands.filter(command=>command.op==='messages.reply')).toHaveLength(0);
    await composer.getByRole('textbox',{name:'Message',exact:true}).press('Enter');
    await expect.poll(()=>f.commands.filter(command=>command.op==='messages.reply').length).toBe(1);
    expect(f.commands.find(command=>command.op==='messages.reply').args).toEqual({id:latest,part_index:part,text:'[ab-test actions] keyboard reply'});
});

test('WEB-MSG-REPLY REVEAL Home then Command-T realizes the latest captured row again',async ({page,context})=>{
    const f=await fixture(page,context);
    await page.goto('/messages/'+encodeURIComponent(chat)+'/'+encodeURIComponent(f.recordID!));
    const field=page.getByRole('textbox',{name:'Message',exact:true});await field.fill('[ab-test actions] unsent reveal draft');
    await expect(page.getByRole('button',{name:'Send message',exact:true})).toBeVisible();
    await expect(page.locator('.messages-transcript')).toHaveAttribute('data-window-end',/^msg_/u);
    const latest=await page.locator('.messages-transcript').getAttribute('data-window-end');
    await page.goto('/messages/'+encodeURIComponent(chat)+'/'+encodeURIComponent(latest!));
    await expect(page.locator('.messages-transcript')).toHaveAttribute('aria-busy','false');
    await page.getByRole('textbox',{name:'Message',exact:true}).fill('[ab-test actions] unsent reveal draft');
    await expect(page.getByRole('button',{name:'Send message',exact:true})).toBeVisible();
    await page.getByRole('listbox',{name:'Messages transcript',exact:true}).focus();await page.keyboard.press('Home');
    await expect(page.locator('[data-message="'+latest+'"]')).toHaveCount(0);
    await page.keyboard.press('Meta+t');
    await expect(page.getByRole('dialog',{name:'Message actions',exact:true})).toBeVisible();
    const anchor=page.locator('.message-action-anchor');await expect(anchor).toBeVisible();
    expect(await anchor.evaluate(element=>element.closest('[data-message]')?.getAttribute('data-message'))).toBe(latest);
    expect(f.commands.filter(command=>command.op==='messages.react'||command.op==='messages.reply')).toHaveLength(0);
});
test('WEB-MSG-REPLY INACTIVE a hidden native stack member is revealed before its keyboard picker opens',async ({page,context,request})=>{
    test.skip(!test.info().project.name.endsWith('-phone'),'Desktop stack cells are all active.');
    const f=await fixture(page,context),ids=await control(request,'/demo-fixtures');
    const leading=ids['demo-message-00-000042'],member=ids['demo-message-00-000045'];
    await page.goto('/messages/'+encodeURIComponent(chat)+'/'+encodeURIComponent(leading));
    const cell=page.locator('.media-stack-cell[data-message="'+member+'"]');await expect(cell).toHaveAttribute('inert','');
    await page.getByRole('textbox',{name:'Message',exact:true}).fill('[ab-test actions] unsent hidden-cell draft');
    await expect(page.getByRole('button',{name:'Send message',exact:true})).toBeVisible();
    f.useLatest(member);await page.keyboard.press('Meta+t');
    await expect(page.getByRole('dialog',{name:'Message actions',exact:true})).toBeVisible();
    const anchor=page.locator('.message-action-anchor');await expect(anchor).toBeVisible();
    expect(await anchor.evaluate(element=>element.closest('[data-message]')?.getAttribute('data-message'))).toBe(member);
    expect(f.commands.filter(command=>command.op==='messages.react')).toHaveLength(0);
});
