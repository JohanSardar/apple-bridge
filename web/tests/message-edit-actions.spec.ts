import { test, expect, type BrowserContext, type Page } from '@playwright/test';
import { control } from './session';
let cookies: Parameters<BrowserContext['addCookies']>[0] = [], chat = '';
test.beforeAll(async ({request}) => { cookies = [(await control(request,'/session')).cookie]; chat = (await control(request,'/demo-fixtures'))['message-chat']; });
async function fixture(page:Page,context:BrowserContext,holdProof=false,mutate?:(record:any)=>void) {
    await page.goto(new URL(cookies[0].url!).origin);
    await expect(page.getByRole('heading',{name:'Sign in with a passkey',exact:true})).toBeVisible();
    await context.addCookies(cookies);
    const commands:any[] = [], statuses:any[] = [], resolves:string[] = [], proofs:(()=>void)[] = [];
    const commandViews=new Map<string,string>(); let holdResults=false,releaseOnResolve=false, close=()=>{};
    await page.routeWebSocket('**/web/v1/live',socket => {
        const server = socket.connectToServer(); close=()=>{void socket.close({code:4000,reason:'fixture_transport_loss'});};
        socket.onMessage(data => { const frame = JSON.parse(String(data)); if(frame.t === 'cmd')commands.push(frame); if(frame.t==='resolve'){resolves.push(frame.id);if(releaseOnResolve)holdResults=false;} if(frame.t==='sub' && frame.args?.collection==='system.command')commandViews.set(frame.id,frame.args.id); server.send(data); });
        server.onMessage(data => {
            const frame = JSON.parse(String(data));
            if(frame.t === 'cmd_status')statuses.push(frame);
            const resultID=frame.t==='cmd_status' ? frame.id : commandViews.get(frame.sub);
            if(holdResults && commands.some(command => ['messages.edit','messages.unsend'].includes(command.op) && command.id===resultID))return;
            if(holdProof && frame.data?.records?.some((record:any) => record.collection === 'system.source' && record.source === 'messages')) {
                proofs.push(() => socket.send(data));
                for(const record of frame.data.records)if(record.collection === 'system.source' && record.source === 'messages') {
                    record.data.capabilities = record.data.capabilities?.filter((operation:string) => !['messages.edit','messages.unsend'].includes(operation));
                    for(const proof of record.data.coverage?.operations??[])if(['messages.edit','messages.unsend'].includes(proof.operation)){proof.proven=false;proof.reason='Waiting for verified change capability';}
                }
                socket.send(JSON.stringify(frame)); return;
            }
            if(mutate){const visit=(value:any)=>{if(!value||typeof value!=='object')return;if(value.data&&typeof value.data==='object'&&(value.collection==='messages.message'||value.collection==='messages.chat'))mutate(value);for(const child of Object.values(value))if(child&&typeof child==='object')Array.isArray(child)?child.forEach(visit):visit(child);};visit(frame);socket.send(JSON.stringify(frame));}else socket.send(data);
        });
    });
    await page.goto('/__fixtures/message-actions?chat='+encodeURIComponent(chat));
    await page.getByRole('button',{name:'Create self fixture message',exact:true}).click();
    const id = page.getByLabel('Created fixture message');
    await expect(id).not.toBeEmpty();
    const recordID = await id.textContent(), target = page.locator('[data-message="'+recordID+'"]');
    await expect(target).toBeVisible();
    return {commands,statuses,resolves,recordID,target,close:()=>close(),holdResults:()=>{holdResults=true;},releaseResults:()=>{holdResults=false;},releaseOnResolve:()=>{releaseOnResolve=true;},releaseProof:() => {holdProof=false;proofs.splice(0).forEach(release => release());}};
}
test('WEB-MSG-EDIT WEB-MSG-EDIT-PLATE trusted Save edits the exact captured part with its captured version',async ({page,context})=>{
    const f=await fixture(page,context);
    await page.goto('/messages/'+encodeURIComponent(chat)+'/'+encodeURIComponent(f.recordID!));
    const row=page.locator('.message-row[data-message="'+f.recordID+'"]');
    await row.locator('.bubble[data-part="0"]').click({button:'right'});
    const menu=page.getByRole('dialog',{name:'Message actions',exact:true});
    const version=Number(await menu.getAttribute('data-message-version'));
    await expect(menu.getByRole('button',{name:'Edit',exact:true})).toBeVisible();
    await menu.getByRole('button',{name:'Edit',exact:true}).click();
    const field=page.getByRole('textbox',{name:'Edit message',exact:true});
    await expect(field).toHaveValue('[ab-test actions] fresh self message');
    await expect(field).toBeFocused();
    await page.getByRole('form',{name:'Edit message',exact:true}).evaluate(form=>Promise.all(form.getAnimations().map(animation=>animation.finished)));
    await expect(page.getByRole('button',{name:'Cancel edit',exact:true})).toBeInViewport({ratio:1});
    await expect(page.getByRole('button',{name:'Save edit',exact:true})).toBeInViewport({ratio:1});
    const theme=await page.evaluate(()=>document.documentElement.dataset.theme);
    await page.screenshot({path:'../lab/evidence/private/web/compose-edit-'+test.info().project.name+'-light.png'});
    await page.evaluate(()=>{document.documentElement.dataset.theme='dark';});
    await page.screenshot({path:'../lab/evidence/private/web/compose-edit-'+test.info().project.name+'-dark.png'});
    await page.evaluate(theme=>{if(theme)document.documentElement.dataset.theme=theme;else delete document.documentElement.dataset.theme;},theme);
    expect(f.commands.filter(command=>command.op==='messages.edit')).toHaveLength(0);
    await field.fill('[ab-test actions] edited by fresh input');
    await page.getByRole('button',{name:'Save edit',exact:true}).evaluate(button=>button.dispatchEvent(new MouseEvent('click',{bubbles:true})));
    expect(f.commands.filter(command=>command.op==='messages.edit')).toHaveLength(0);
    await page.getByRole('button',{name:'Save edit',exact:true}).click();
    await expect.poll(()=>f.commands.filter(command=>command.op==='messages.edit').length).toBe(1);
    const command=f.commands.find(command=>command.op==='messages.edit');
    expect(command.args).toEqual({id:f.recordID,part_index:0,text:'[ab-test actions] edited by fresh input'});
    expect(command.expected_version).toBe(version);
    await expect(row).toContainText('[ab-test actions] edited by fresh input');
    await expect(row).toHaveCount(1);
});
test('WEB-MSG-EDIT WEB-MSG-UNSEND trusted selection unsends the exact captured part once',async ({page,context})=>{
    const f=await fixture(page,context);
    await page.goto('/messages/'+encodeURIComponent(chat)+'/'+encodeURIComponent(f.recordID!));
    const row=page.locator('.message-row[data-message="'+f.recordID+'"]');
    await row.locator('.bubble[data-part="0"]').click({button:'right'});
    const menu=page.getByRole('dialog',{name:'Message actions',exact:true});
    const version=Number(await menu.getAttribute('data-message-version'));
    await expect(menu.getByRole('button',{name:'Undo Send',exact:true})).toBeVisible();
    await menu.getByRole('button',{name:'Undo Send',exact:true}).click();
    await expect.poll(()=>f.commands.filter(command=>command.op==='messages.unsend').length).toBe(1);
    const command=f.commands.find(command=>command.op==='messages.unsend');
    expect(command.args).toEqual({id:f.recordID,part_index:0});expect(command.expected_version).toBe(version);
    await expect.poll(()=>f.statuses.some(frame=>frame.id===command.id&&['observed','delivered'].includes(frame.command?.state??frame.status))).toBe(true);
    await expect(row).toContainText(/unsent/u);
    expect(f.commands.filter(command=>command.op==='messages.unsend')).toHaveLength(1);
});

async function openMain(page:Page,f:Awaited<ReturnType<typeof fixture>>){
    await page.goto('/messages/'+encodeURIComponent(chat)+'/'+encodeURIComponent(f.recordID!));
    const row=page.locator('.message-row[data-message="'+f.recordID+'"]');await expect(row).toBeVisible();
    await row.locator('.bubble[data-part="0"]').click({button:'right'});
    return {row,menu:page.getByRole('dialog',{name:'Message actions',exact:true})};
}
test('WEB-MSG-EDIT cancel and IME leave the captured message untouched',async({page,context})=>{
    const f=await fixture(page,context),{row,menu}=await openMain(page,f);
    await menu.getByRole('button',{name:'Edit',exact:true}).click();
    const field=page.getByRole('textbox',{name:'Edit message',exact:true});
    await field.fill('[ab-test actions] not saved');
    await field.dispatchEvent('keydown',{key:'Enter',isComposing:true,keyCode:229});
    expect(f.commands.filter(command=>command.op==='messages.edit')).toHaveLength(0);
    await field.press('Shift+Enter');await expect(field).toHaveValue('[ab-test actions] not saved\n');
    await field.press('Escape');await expect(field).toHaveCount(0);
    await expect(row).toContainText('[ab-test actions] fresh self message');
    expect(f.commands.filter(command=>command.op==='messages.edit')).toHaveLength(0);
});
test('WEB-MSG-EDIT WEB-MSG-UNSEND fresh proof reveals changes; synthetic Undo Send never dispatches',async({page,context})=>{
    const f=await fixture(page,context,true),{menu}=await openMain(page,f);
    await expect(menu.getByRole('button',{name:'Edit',exact:true})).toHaveCount(0);
    await expect(menu.getByRole('button',{name:'Undo Send',exact:true})).toHaveCount(0);
    f.releaseProof();await expect(menu.getByRole('button',{name:'Edit',exact:true})).toBeVisible();
    await expect(menu.getByRole('button',{name:'Undo Send',exact:true})).toBeVisible();
    await menu.getByRole('button',{name:'Undo Send',exact:true}).dispatchEvent('click');
    expect(f.commands.filter(command=>command.op==='messages.unsend')).toHaveLength(0);
});
for(const [name,change] of [
    ['received',(record:any)=>{if(record.collection==='messages.message')record.data.is_from_me=false;}],
    ['future',(record:any)=>{if(record.collection==='messages.message')record.data.sent_at=new Date(Date.now()+60000).toISOString();}],
    ['expired',(record:any)=>{if(record.collection==='messages.message')record.data.sent_at=new Date(Date.now()-901000).toISOString();}],
    ['sms',(record:any)=>{if(record.collection==='messages.chat')record.data.service='SMS';}],
] as const)test('WEB-MSG-EDIT WEB-MSG-UNSEND '+name+' capture hides both actions',async({page,context})=>{
    const f=await fixture(page,context,false,change),{menu}=await openMain(page,f);
    await expect(menu).toBeVisible();await expect(menu.getByRole('button',{name:'Edit',exact:true})).toHaveCount(0);await expect(menu.getByRole('button',{name:'Undo Send',exact:true})).toHaveCount(0);
    expect(f.commands.filter(command=>['messages.edit','messages.unsend'].includes(command.op))).toHaveLength(0);
});
test('WEB-MSG-EDIT WEB-MSG-UNSEND five edits and elapsed undo window hide their respective actions',async({page,context})=>{
    const f=await fixture(page,context,false,record=>{if(record.collection==='messages.message'){record.data.sent_at=new Date(Date.now()-121000).toISOString();record.data.edit_history=Array.from({length:5},()=>({text:'[ab-test actions] previous version'}));}}),{menu}=await openMain(page,f);
    await expect(menu).toBeVisible();await expect(menu.getByRole('button',{name:'Edit',exact:true})).toHaveCount(0);await expect(menu.getByRole('button',{name:'Undo Send',exact:true})).toHaveCount(0);
});
test('WEB-MSG-EDIT Check reconciles a lost edit result by original ID without another edit',async({page,context,request})=>{
    const f=await fixture(page,context),{menu}=await openMain(page,f);await menu.getByRole('button',{name:'Edit',exact:true}).click();
    await page.getByRole('textbox',{name:'Edit message',exact:true}).fill('[ab-test actions] edit response lost');f.holdResults();
    await page.getByRole('button',{name:'Save edit',exact:true}).click();
    await expect.poll(()=>f.statuses.some(frame=>frame.id===f.commands.find(command=>command.op==='messages.edit')?.id&&['observed','delivered'].includes(frame.command?.state??frame.status))).toBe(true);
    const original=f.commands.find(command=>command.op==='messages.edit');f.close();
    const editor=page.getByRole('form',{name:'Edit message',exact:true});
    await expect(editor).toContainText('Not sure if the message changed.');
    await expect(page.getByRole('button',{name:'Save edit',exact:true})).toBeDisabled();
    f.releaseOnResolve();const check=editor.getByRole('button',{name:'Check',exact:true});await expect(check).toBeEnabled();await check.click();
    await expect.poll(()=>f.resolves.filter(id=>id===original.id).length).toBe(1);
    await expect(editor).toHaveCount(0);
    expect(f.commands.filter(command=>command.op==='messages.edit').map(command=>command.id)).toEqual([original.id]);
    const effect=await request.post(process.env.AB_WEB_CONTROL!+'/command-effect',{headers:{'X-Bridge-Lab':process.env.AB_WEB_CONTROL_KEY!},data:{command_id:original.id}});expect(effect.ok()).toBe(true);expect((await effect.json()).count).toBe(1);
});
test('WEB-MSG-EDIT keyboard opens the last owned message and Enter saves once',async({page,context})=>{
    const f=await fixture(page,context);await page.goto('/messages/'+encodeURIComponent(chat)+'/'+encodeURIComponent(f.recordID!));
    await expect(page.locator('.messages-transcript')).toHaveAttribute('aria-busy','false');
    await expect(page.locator('[data-message="'+f.recordID+'"]')).toBeVisible();await page.keyboard.press('Meta+e');
    const field=page.getByRole('textbox',{name:'Edit message',exact:true});await expect(field).toHaveValue('[ab-test actions] fresh self message');
    await field.fill('[ab-test actions] keyboard edit');await field.press('Enter');
    await expect.poll(()=>f.commands.filter(command=>command.op==='messages.edit').length).toBe(1);
    expect(f.commands.find(command=>command.op==='messages.edit').args.id).toBe(f.recordID);
});
test('WEB-MSG-EDIT WEB-MSG-UNSEND keyboard undo preserves draft undo and unsends only a fresh native input',async({page,context})=>{
    const f=await fixture(page,context);await page.goto('/messages/'+encodeURIComponent(chat)+'/'+encodeURIComponent(f.recordID!));
    const composer=page.getByRole('textbox',{name:'Message',exact:true});await expect(page.locator('[data-message="'+f.recordID+'"]')).toBeVisible();
    await composer.fill('[ab-test actions] draft undo');await composer.press('Meta+z');expect(f.commands.filter(command=>command.op==='messages.unsend')).toHaveLength(0);
    await composer.blur();await page.keyboard.press('Meta+z');
    await expect.poll(()=>f.commands.filter(command=>command.op==='messages.unsend').length).toBe(1);
    expect(f.commands.find(command=>command.op==='messages.unsend').args).toEqual({id:f.recordID,part_index:0});
});
test('WEB-MSG-EDIT reader Reply closes the reader and focuses the real main Composer',async({page,context,request})=>{
    const f=await fixture(page,context),ids=await control(request,'/demo-fixtures'),root=ids['demo-message-00-000006'];
    await page.goto('/messages/'+encodeURIComponent(chat)+'/'+encodeURIComponent(root));
    const main=page.locator('.message-pane>.messages-transcript'),trigger=main.locator('[data-message="'+root+'"] button.reply-count');
    await expect(trigger).toBeVisible();await trigger.click();
    const reader=page.getByRole('dialog',{name:'Replies',exact:true});await expect(reader).toBeVisible();
    await reader.locator('[data-message="'+root+'"] .bubble[data-part="0"]').click({button:'right'});
    const menu=page.getByRole('dialog',{name:'Message actions',exact:true});await expect(menu.getByRole('button',{name:'Edit',exact:true})).toHaveCount(0);await expect(menu.getByRole('button',{name:'Undo Send',exact:true})).toHaveCount(0);
    await menu.getByRole('button',{name:'Reply',exact:true}).click();
    await expect(reader).toHaveCount(0);await expect(trigger).not.toBeFocused();
    const composer=page.getByRole('region',{name:'Message composer',exact:true});await expect(composer.getByRole('region',{name:'Replying to message',exact:true})).toBeVisible();await expect(composer.getByRole('textbox',{name:'Message',exact:true})).toBeFocused();
    expect(f.commands.filter(command=>command.op==='messages.reply')).toHaveLength(0);
});

test('WEB-MSG-EDIT WEB-MSG-UNSEND an elapsed undo window still permits Edit before fifteen minutes',async({page,context})=>{
    const f=await fixture(page,context,false,record=>{if(record.collection==='messages.message')record.data.sent_at=new Date(Date.now()-121000).toISOString();}),{menu}=await openMain(page,f);
    await expect(menu.getByRole('button',{name:'Edit',exact:true})).toBeVisible();await expect(menu.getByRole('button',{name:'Undo Send',exact:true})).toHaveCount(0);
});
test('WEB-MSG-EDIT WEB-MSG-UNSEND an uncertain Undo Send is reconciled without a second unsend',async({page,context,request})=>{
    const f=await fixture(page,context),{menu}=await openMain(page,f);f.holdResults();await menu.getByRole('button',{name:'Undo Send',exact:true}).click();
    await expect.poll(()=>f.statuses.some(frame=>frame.id===f.commands.find(command=>command.op==='messages.unsend')?.id&&['observed','delivered'].includes(frame.command?.state??frame.status))).toBe(true);
    const original=f.commands.find(command=>command.op==='messages.unsend');f.close();
    const notice=page.getByRole('status',{name:'Uncertain delivery',exact:true});await expect(notice).toContainText('Not sure if the message changed');
    f.releaseOnResolve();const check=notice.getByRole('button',{name:'Check',exact:true});await expect(check).toBeEnabled();await check.click();
    await expect.poll(()=>f.resolves.filter(id=>id===original.id).length).toBe(1);await expect(notice).toHaveCount(0);
    expect(f.commands.filter(command=>command.op==='messages.unsend').map(command=>command.id)).toEqual([original.id]);const effect=await request.post(process.env.AB_WEB_CONTROL!+'/command-effect',{headers:{'X-Bridge-Lab':process.env.AB_WEB_CONTROL_KEY!},data:{command_id:original.id}});expect(effect.ok()).toBe(true);expect((await effect.json()).count).toBe(1);
});
