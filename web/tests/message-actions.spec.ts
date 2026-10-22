import { test, expect, type BrowserContext, type Page } from '@playwright/test';
import { control } from './session';
let cookies: Parameters<BrowserContext['addCookies']>[0] = [], chat = '';
test.beforeAll(async ({request}) => { cookies = [(await control(request,'/session')).cookie]; chat = (await control(request,'/demo-fixtures'))['message-chat']; });
async function fixture(page:Page,context:BrowserContext,holdProof=false) {
    await page.goto(new URL(cookies[0].url!).origin);
    await expect(page.getByRole('heading',{name:'Sign in with a passkey',exact:true})).toBeVisible();
    await context.addCookies(cookies);
    const commands:any[] = [], statuses:any[] = [], resolves:string[] = [], proofs:(()=>void)[] = [];
    const commandViews=new Map<string,string>(); let holdResults=false, close=()=>{};
    await page.routeWebSocket('**/web/v1/live',socket => {
        const server = socket.connectToServer(); close=()=>{void socket.close({code:4000,reason:'fixture_transport_loss'});};
        socket.onMessage(data => { const frame = JSON.parse(String(data)); if(frame.t === 'cmd')commands.push(frame); if(frame.t==='resolve')resolves.push(frame.id); if(frame.t==='sub' && frame.args?.collection==='system.command')commandViews.set(frame.id,frame.args.id); server.send(data); });
        server.onMessage(data => {
            const frame = JSON.parse(String(data));
            if(frame.t === 'cmd_status')statuses.push(frame);
            const resultID=frame.t==='cmd_status' ? frame.id : commandViews.get(frame.sub);
            if(holdResults && commands.some(command => command.op==='messages.react' && command.id===resultID))return;
            if(holdProof && frame.data?.records?.some((record:any) => record.collection === 'system.source' && record.source === 'messages')) {
                proofs.push(() => socket.send(data));
                for(const record of frame.data.records)if(record.collection === 'system.source' && record.source === 'messages') {
                    record.data.capabilities = record.data.capabilities?.filter((operation:string) => operation !== 'messages.react');
                    const proof = record.data.coverage?.operations?.find((entry:any) => entry.operation === 'messages.react');
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
    return {commands,statuses,resolves,recordID,target,close:()=>close(),holdResults:()=>{holdResults=true;},releaseResults:()=>{holdResults=false;},releaseProof:() => {holdProof=false;proofs.splice(0).forEach(release => release());}};
}
test('WEB-MSG-REACT native context menu opens six tapbacks and one trusted selection dispatches once',async ({page,context}) => {
    const f = await fixture(page,context);
    await f.target.click({button:'right'});
    const menu = page.getByRole('dialog',{name:'Message actions',exact:true});
    await expect(menu).toBeVisible();
    for(const label of ['Love','Like','Dislike','Laugh','Emphasize','Question'])await expect(menu.getByRole('button',{name:label,exact:true})).toBeVisible();
    expect(f.commands.filter(command => command.op === 'messages.react')).toHaveLength(0);
    await menu.getByRole('button',{name:'Love',exact:true}).evaluate(button => button.dispatchEvent(new MouseEvent('click',{bubbles:true})));
    await page.evaluate(() => new Promise(requestAnimationFrame));
    expect(f.commands.filter(command => command.op==='messages.react')).toHaveLength(0);
    await menu.getByRole('button',{name:'Love',exact:true}).click();
    await expect.poll(() => f.commands.filter(command => command.op === 'messages.react').length).toBe(1);
    const command = f.commands.find(command => command.op === 'messages.react');
    expect(command.args).toEqual({id:f.recordID,part_index:0,reaction:'love',remove:false});
    await expect(menu).toHaveCount(0);
    await expect.poll(() => f.statuses.some(frame => frame.id === command.id && ['observed','delivered'].includes(frame.status))).toBe(true);
});
test('WEB-MSG-REACT REMOVE a matching own tapback removes it and emoji search sends the exact selected emoji',async ({page,context}) => {
    const f = await fixture(page,context);
    const menu = page.getByRole('dialog',{name:'Message actions',exact:true});
    await f.target.click({button:'right'}); await menu.getByRole('button',{name:'Love',exact:true}).click();
    await expect.poll(() => f.commands.filter(command => command.op === 'messages.react').length).toBe(1);
    await f.target.click({button:'right'});
    await expect(menu.getByRole('button',{name:'Love',exact:true})).toHaveAttribute('aria-pressed','true');
    await menu.getByRole('button',{name:'Love',exact:true}).click();
    await expect.poll(() => f.commands.filter(command => command.op === 'messages.react').length).toBe(2);
    expect(f.commands.filter(command => command.op === 'messages.react')[1].args).toEqual({id:f.recordID,part_index:0,reaction:'love',remove:true});
    await f.target.click({button:'right'});
    await menu.getByRole('button',{name:'More reactions',exact:true}).click();
    await menu.getByRole('searchbox',{name:'Search reactions',exact:true}).fill('party');
    expect(f.commands.filter(command => command.op === 'messages.react')).toHaveLength(2);
    await menu.getByRole('button',{name:'Party popper',exact:true}).click();
    await expect.poll(() => f.commands.filter(command => command.op === 'messages.react').length).toBe(3);
    expect(f.commands.filter(command => command.op === 'messages.react')[2].args).toEqual({id:f.recordID,part_index:0,reaction:'emoji',emoji:'🎉',remove:false});
});
test('WEB-MSG-REACT PROOF unproven reactions stay hidden until the live proof arrives without dispatch',async ({page,context}) => {
    const f = await fixture(page,context,true);
    await f.target.click({button:'right'});
    const menu = page.getByRole('dialog',{name:'Message actions',exact:true});
    await expect(menu).toBeVisible();
    await expect(menu.getByRole('button',{name:'Love',exact:true})).toHaveCount(0);
    f.releaseProof();
    await expect(menu.getByRole('button',{name:'Love',exact:true})).toBeVisible();
    expect(f.commands.filter(command => command.op === 'messages.react')).toHaveLength(0);
    await page.keyboard.press('Escape'); await expect(menu).toHaveCount(0);
    await expect(f.target).toBeFocused();
    expect(f.commands.filter(command => command.op === 'messages.react')).toHaveLength(0);
});

test('WEB-MSG-REACT AMBIGUITY a lost result blocks repeated reaction writes until read-only Check recovers the original command',async ({page,context,request}) => {
    const f=await fixture(page,context), menu=page.getByRole('dialog',{name:'Message actions',exact:true});
    f.holdResults();
    await f.target.click({button:'right'}); await menu.getByRole('button',{name:'Love',exact:true}).click();
    await expect.poll(() => f.statuses.some(frame => frame.id===f.commands.find(command => command.op==='messages.react')?.id && frame.command?.state==='observed')).toBe(true);
    const original=f.commands.find(command => command.op==='messages.react');
    f.close();
    await expect(page.getByRole('status',{name:'Reaction status'})).toContainText('1 reaction may not have changed');
    await f.target.click({button:'right'});
    await expect(menu.getByText('Not sure if the reaction changed.',{exact:true})).toBeVisible();
    await expect(menu.getByRole('button',{name:'Love',exact:true})).toBeDisabled();
    f.releaseResults();
    await menu.getByRole('button',{name:'Check',exact:true}).click();
    await expect(menu.getByText('Not sure if the reaction changed.',{exact:true})).toHaveCount(0);
    await expect(menu.getByRole('button',{name:'Love',exact:true})).toHaveAttribute('aria-pressed','true');
    await expect(page.getByRole('status',{name:'Reaction status'})).toHaveCount(0);
    expect(f.commands.filter(command => command.op==='messages.react').map(command => command.id)).toEqual([original.id]);
    expect(f.resolves).toEqual([original.id]);
    const effect=await request.post(process.env.AB_WEB_CONTROL!+'/command-effect',{headers:{'X-Bridge-Lab':process.env.AB_WEB_CONTROL_KEY!},data:{command_id:original.id}});
    expect(effect.ok()).toBe(true); expect((await effect.json()).count).toBe(1);
});

test('WEB-MSG-REACT APP the actual transcript opens the guarded picker for a captured native part',async ({page,context}) => {
    const f=await fixture(page,context);
    await page.goto('/messages/'+encodeURIComponent(chat)+'/'+encodeURIComponent(f.recordID!));
    const row=page.locator('.message-row[data-message="'+f.recordID+'"]');
    await expect(row).toBeVisible();
    await row.locator('.bubble[data-part="0"]').click({button:'right'});
    const menu=page.getByRole('dialog',{name:'Message actions',exact:true});
    await expect(menu).toBeVisible();
    await page.evaluate(()=>Promise.all(document.getAnimations().filter(animation=>animation.effect?.getTiming().iterations!==Infinity).map(animation=>animation.finished.catch(()=>{}))));
    const theme=await page.evaluate(()=>document.documentElement.dataset.theme);
    await page.screenshot({path:'../lab/evidence/private/web/compose-actions-'+test.info().project.name+'-light.png'});
    await page.evaluate(()=>{document.documentElement.dataset.theme='dark';});
    await page.screenshot({path:'../lab/evidence/private/web/compose-actions-'+test.info().project.name+'-dark.png'});
    await page.evaluate(theme=>{if(theme)document.documentElement.dataset.theme=theme;else delete document.documentElement.dataset.theme;},theme);
    await menu.getByRole('button',{name:'Question',exact:true}).click();
    await expect.poll(()=>f.commands.filter(command=>command.op==='messages.react').length).toBe(1);
    expect(f.commands.find(command=>command.op==='messages.react').args).toEqual({id:f.recordID,part_index:0,reaction:'question',remove:false});
    await expect(row.getByRole('button',{name:/reacted ❓/u})).toBeVisible();
    const latest=await page.locator('.messages-transcript').getAttribute('data-window-end');
    const other=page.locator('.message-row[data-message]');
    await other.evaluateAll((rows,latest)=>{const row=rows.find(row=>row.getAttribute('data-message')!==latest) as HTMLElement|undefined;if(row){row.tabIndex=-1;row.focus();}},latest);
    await page.keyboard.press('Meta+t');
    await expect(menu).toBeVisible();
    await expect.poll(()=>page.locator('.message-action-anchor').evaluate(element=>element.closest('[data-message]')?.getAttribute('data-message'))).toBe(latest);
    await expect(menu.getByRole('button',{name:'Question',exact:true})).toHaveAttribute('aria-pressed',latest===f.recordID?'true':'false');
    await page.keyboard.press('Escape');
    expect(f.commands.filter(command=>command.op==='messages.react')).toHaveLength(1);
});

test('WEB-MSG-REACT TOUCH a trusted long press opens once and touch release cannot choose a reaction',async ({page,context,browserName}) => {
    test.skip(browserName!=='chromium'||!test.info().project.name.endsWith('phone'),'Trusted held touch uses the Chromium protocol; native context-menu and keyboard remain cross-browser.');
    const f=await fixture(page,context);
    await page.goto('/messages/'+encodeURIComponent(chat)+'/'+encodeURIComponent(f.recordID!));
    const bubble=page.locator('.message-row[data-message="'+f.recordID+'"] .bubble[data-part="0"]');
    await expect(bubble).toBeVisible();
    const box=await bubble.boundingBox();expect(box).not.toBeNull();
    const session=await context.newCDPSession(page);
    await session.send('Input.dispatchTouchEvent',{type:'touchStart',touchPoints:[{x:box!.x+box!.width/2,y:box!.y+box!.height/2}]});
    const menu=page.getByRole('dialog',{name:'Message actions',exact:true});
    await expect(menu).toBeVisible();
    await session.send('Input.dispatchTouchEvent',{type:'touchEnd',touchPoints:[]});
    await page.evaluate(()=>new Promise(requestAnimationFrame));
    await expect(menu).toBeVisible();
    expect(f.commands.filter(command=>command.op==='messages.react')).toHaveLength(0);
    await menu.getByRole('button',{name:'Like',exact:true}).click();
    await expect.poll(()=>f.commands.filter(command=>command.op==='messages.react').length).toBe(1);
    expect(f.commands.find(command=>command.op==='messages.react').args).toEqual({id:f.recordID,part_index:0,reaction:'like',remove:false});
    await session.detach();
});

test('WEB-MSG-REACT COPY copies only the selected captured text and never dispatches',async ({page,context}) => {
    const f=await fixture(page,context), before=f.commands.length;
    await page.evaluate(() => { (window as any).__actionCopies=[]; Object.defineProperty(navigator,'clipboard',{configurable:true,value:{writeText:async (text:string) => { (window as any).__actionCopies.push(text); }}}); });
    await f.target.click({button:'right'});
    const button=page.getByRole('dialog',{name:'Message actions',exact:true}).getByRole('button',{name:'Copy',exact:true});
    await button.evaluate(button=>button.dispatchEvent(new MouseEvent('click',{bubbles:true})));
    expect(await page.evaluate(()=>(window as any).__actionCopies)).toEqual([]);
    await button.click();
    expect(await page.evaluate(() => (window as any).__actionCopies)).toEqual(['[ab-test actions] fresh self message']);
    expect(f.commands).toHaveLength(before);
});
