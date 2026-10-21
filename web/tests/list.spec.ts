import { expect } from '@playwright/test';
import { test } from './list-session';
import { bootstrap, control } from './session';
import { mkdir } from 'node:fs/promises';
test.beforeEach(async ({page}) => {await page.goto('/messages');await expect(page.getByRole('listbox',{name:'Conversations'})).toBeVisible();});
test('WEB-MSG-LIST rows, avatar anatomy, ordering, live changes and selection',async({page,request},info)=>{
    const rows=page.locator('.conversation-row');
    await expect(rows.first()).toContainText('Maya Chen');
    await expect(rows.first().locator('.avatar')).toHaveCSS('width',info.project.use.viewport?.width===390?'52px':'40px');
    await expect(rows.first().locator('.avatar')).toContainText('MC');
    await expect(rows.first().locator('time')).toHaveText(/^\d{1,2}:\d{2}$/);
    if(info.project.use.viewport?.width===390){const chevron=rows.first().locator('.conversation-chevron');await expect(chevron).toBeVisible();expect(await chevron.evaluate(element=>new DOMMatrix(getComputedStyle(element).transform).a)).toBe(-1);await page.evaluate(()=>document.documentElement.dir='rtl');expect(await chevron.evaluate(element=>new DOMMatrix(getComputedStyle(element).transform).a)).toBe(1);await page.evaluate(()=>document.documentElement.dir='ltr');}
    await expect(rows.filter({hasText:'Archived fixture'})).toHaveCount(0);
    await expect(page.locator('.conversation-row').filter({hasText:'Weekend Crew'}).locator('.avatar-member')).toHaveCount(3);
    await expect(page.getByText('Attachment: 2 Images',{exact:true})).toBeVisible();
    await expect(page.getByText('Loved “See you soon”',{exact:true})).toBeVisible();
    await expect(page.getByLabel('Notifications silenced').first()).toBeVisible();
    const photo=rows.filter({hasText:'Sam Rivera'}).locator('img');await expect(photo).toBeVisible();await expect(photo).toHaveAttribute('src',/variant=thumb/);await expect.poll(()=>photo.evaluate((image:HTMLImageElement)=>image.complete&&image.naturalWidth>0)).toBe(true);
    const group=rows.filter({hasText:'Conversation 010'});await expect(group.locator('.conversation-preview')).toHaveText('Taylor Quinn named the conversation “Friday plans”');await expect(group.locator('img')).toBeVisible();await expect(group.locator('.avatar-member')).toHaveCount(0);
    await expect(rows.filter({hasText:'Conversation 011'}).locator('.conversation-preview')).toHaveText('You unsent a message');
    const emptyGroup=rows.filter({hasText:'Conversation 012'}).locator('.avatar');await expect(emptyGroup).toContainText('C0');await expect(emptyGroup).toHaveCSS('background-image',/linear-gradient/);
    await expect(rows.filter({hasText:'Conversation 013'}).locator('.avatar-member')).toHaveCount(0);
    const pair=rows.filter({hasText:'Conversation 014'}).locator('.avatar-member');await expect(pair).toHaveCount(2);const tops=await pair.evaluateAll(elements=>elements.map(element=>element.getBoundingClientRect().top));expect(tops[1]).toBeGreaterThan(tops[0]);
    const listViewport=page.getByRole('listbox',{name:'Conversations'});
    for(const [title,label] of [['Conversation 005',/^Yesterday$/],['Conversation 006',/^(Monday|Tuesday|Wednesday|Thursday|Friday|Saturday|Sunday)$/],['Conversation 007',/^\d{1,2}\/\d{1,2}\/\d{2}$/]] as const){
        const row=rows.filter({hasText:title});
        for(let attempt=0;attempt<100&&!await row.count();attempt++){await listViewport.evaluate(element=>element.scrollTop+=Math.max(200,element.clientHeight*.8));await page.evaluate(()=>new Promise<void>(resolve=>requestAnimationFrame(()=>requestAnimationFrame(()=>resolve()))));}
        await expect(row.locator('time')).toHaveText(label);await listViewport.evaluate(element=>element.scrollTop=0);await expect(rows.first()).toContainText('Maya Chen');
    }
    await expect(rows.first().locator('.unread-dot')).toBeVisible();
    const target=rows.filter({hasText:'Weekend Crew'}); const id=await target.getAttribute('data-chat');
    await target.click(); await expect(page).toHaveURL(new RegExp('/messages/'+id+'$'));
    if(info.project.use.viewport?.width===1440){
        await expect(target.locator('..')).toHaveAttribute('aria-selected','true');
        await page.evaluate(()=>{Object.defineProperty(document,'hasFocus',{configurable:true,value:()=>true});window.dispatchEvent(new Event('focus'));});
        await expect(page.locator('.conversation-list')).not.toHaveClass(/window-inactive/);
        const fill=await target.evaluate(element=>getComputedStyle(element).backgroundColor);
        await page.evaluate(()=>{Object.defineProperty(document,'hasFocus',{configurable:true,value:()=>false});window.dispatchEvent(new Event('blur'));});
        await expect(page.locator('.conversation-list')).toHaveClass(/window-inactive/);
        expect(await target.evaluate(element=>getComputedStyle(element).backgroundColor)).not.toBe(fill);
    }
    await page.goto('/messages');await expect(page.getByRole('listbox',{name:'Conversations'})).toBeVisible();
    await page.evaluate(()=>{(window as any).__rowMotion=[];(window as any).__dotMotion=[];new MutationObserver(changes=>{for(const change of changes){const row=change.target as HTMLElement;if(row.matches('.conversation-row')&&row.style.transform)(window as any).__rowMotion.push({chat:row.dataset.chat,transform:row.style.transform});if(row.matches('.unread-dot')&&row.style.transform)(window as any).__dotMotion.push({chat:(row.closest('.conversation-row') as HTMLElement)?.dataset.chat,transform:row.style.transform});}}).observe(document.querySelector('.conversation-list')!,{subtree:true,attributes:true,attributeFilter:['style']});});
    const mayaID=await rows.first().getAttribute('data-chat'),viewport=page.getByRole('listbox',{name:'Conversations'});
    await viewport.evaluate(element=>element.scrollTop=1000);await expect(page.locator(`[data-chat="${mayaID}"]`)).toHaveCount(0);await viewport.evaluate(element=>element.scrollTop=0);await expect(rows.first()).toContainText('Maya Chen');
    expect(await page.evaluate(id=>(window as any).__dotMotion.some((entry:any)=>entry.chat===id),mayaID)).toBe(false);
    await control(request,'/list-fixture?update=1');
    await expect(rows.first()).toContainText('Weekend Crew');
    await expect(rows.first()).toContainText('The plan is ready');
    await expect.poll(()=>page.evaluate(()=>(window as any).__rowMotion.length)).toBeGreaterThan(0);
    await expect.poll(()=>page.evaluate(id=>(window as any).__dotMotion.some((entry:any)=>entry.chat===id),id)).toBe(true);
    await expect(page.getByRole('button',{name:'New Message',exact:true})).toBeDisabled();
    await expect.poll(()=>rows.evaluateAll(elements=>elements.every(element=>!(element as HTMLElement).style.transform))).toBe(true);
    await expect.poll(()=>rows.first().locator('.unread-dot').evaluate(element=>element.getBoundingClientRect().width)).toBe(8);
    await mkdir('../lab/evidence/private/web',{recursive:true});
    await page.screenshot({path:`../lab/evidence/private/web/list-fixture-${info.project.name}.png`});
    await expect.poll(()=>page.evaluate(async()=>{
        if(!(await indexedDB.databases()).some(db=>db.name==='ab.views'))return 0;
        const db=await new Promise<IDBDatabase>((resolve,reject)=>{const open=indexedDB.open('ab.views');open.onsuccess=()=>resolve(open.result);open.onerror=()=>reject(open.error);});
        const values=await new Promise<any[]>((resolve,reject)=>{const read=db.transaction('views').objectStore('views').getAll();read.onsuccess=()=>resolve(read.result);read.onerror=()=>reject(read.error);});db.close();return Math.max(0,...values.filter(value=>value.data.snapshot_version===1).map(value=>value.data.chats?.length??0));
    })).toBeGreaterThanOrEqual(540);
    await page.routeWebSocket('**/web/v1/live',()=>{});
    await page.addInitScript(()=>{(window as any).__listSkeletonPainted=false;new MutationObserver(()=>requestAnimationFrame(()=>{const skeleton=document.querySelector<HTMLElement>('.list-skeleton');if(skeleton&&skeleton.getBoundingClientRect().height>0)(window as any).__listSkeletonPainted=true;})).observe(document,{childList:true,subtree:true});});
    await page.reload();await expect(viewport).toBeVisible();await expect.poll(async()=>Number(await viewport.locator('[aria-setsize]').first().getAttribute('aria-setsize'))).toBeGreaterThanOrEqual(540);
    await page.waitForTimeout(220);expect(await page.evaluate(()=>(window as any).__listSkeletonPainted)).toBe(false);await expect(page.locator('.list-skeleton')).toHaveCount(0);
});
test('WEB-MSG-KEYBOARD-LIST optional pins, preview bubbles and numbered shortcuts',async({page,request},info)=>{
    await expect(page.getByRole('group',{name:'Pinned conversations'})).toHaveCount(0);
    await control(request,'/list-pins');
    const pins=page.getByRole('group',{name:'Pinned conversations'});
    await expect(pins.getByRole('button')).toHaveCount(3);
    await expect(pins.locator('.pin-preview').first()).toContainText('Are we still on for tomorrow?');
    if(info.project.use.viewport?.width===390){await pins.getByRole('button',{name:'Maya Chen',exact:true}).click();await expect(page).toHaveURL(/messages\/cht_/);return;}
    const modifier=await page.evaluate(()=>/Mac|iPhone|iPad/.test(navigator.platform)?'Meta':'Control');
    await page.keyboard.press(modifier+'+2');await expect(pins.getByRole('button',{name:'Weekend Crew'})).toHaveAttribute('aria-pressed','true');
    await page.keyboard.press(modifier+'+1');await expect(pins.getByRole('button',{name:'Maya Chen'})).toHaveAttribute('aria-pressed','true');
});
test('WEB-MSG-KEYBOARD-LIST arrows, chat cycling and reserved compose',async({page},info)=>{
    test.skip(info.project.use.viewport?.width===390,'Desktop keyboard shortcuts');
    const modifier=await page.evaluate(()=>/Mac|iPhone|iPad/.test(navigator.platform)?'Meta':'Control');
    await page.getByRole('listbox',{name:'Conversations'}).focus();
    await page.keyboard.press('ArrowDown');await expect(page.locator('.conversation-row').first().locator('..')).toHaveAttribute('aria-selected','true');
    await page.keyboard.press('ArrowDown');await expect(page.locator('.conversation-row').nth(1).locator('..')).toHaveAttribute('aria-selected','true');
    await page.keyboard.press('ArrowUp');await expect(page.locator('.conversation-row').first().locator('..')).toHaveAttribute('aria-selected','true');
    await page.keyboard.press('Control+Tab');await expect(page.locator('.conversation-row').nth(1).locator('..')).toHaveAttribute('aria-selected','true');
    await page.keyboard.press('Control+Shift+Tab');await expect(page.locator('.conversation-row').first().locator('..')).toHaveAttribute('aria-selected','true');
    const before=page.url();await page.keyboard.press(modifier+'+n');expect(page.url()).toBe(before);
});

test('WEB-MSG-LIST uncached loading waits before skeletons and clears on live data',async({page,context,request})=>{
    await page.goto('/settings/devices');await page.getByRole('button',{name:'Sign out',exact:true}).click();await expect(page.getByRole('heading',{name:'Sign in with a passkey'})).toBeVisible();await bootstrap(context,request,'public');
    let holding=true;const pending:(()=>void)[]=[];
    await page.routeWebSocket('**/web/v1/live',route=>{const server=route.connectToServer();route.onMessage(message=>server.send(message));server.onMessage(message=>{if(holding)pending.push(()=>route.send(message));else route.send(message);});});
    await page.addInitScript(()=>{const observer=new MutationObserver(()=>{if(!document.querySelector('.conversation-list'))return;observer.disconnect();setTimeout(()=>(window as any).__listEarlySkeleton=Boolean(document.querySelector('.list-skeleton')),100);});observer.observe(document,{subtree:true,childList:true});});
    await page.goto('/messages');await expect.poll(()=>page.evaluate(()=>(window as any).__listEarlySkeleton)).toBe(false);await expect(page.getByLabel('Loading conversations')).toBeVisible();
    holding=false;for(const send of pending.splice(0))send();
    await expect(page.getByRole('listbox',{name:'Conversations'})).toBeVisible();await expect(page.locator('.list-skeleton')).toHaveCount(0);
});
