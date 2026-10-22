import { expect, type Page } from '@playwright/test';
import { test } from './list-session';
import { control } from './session';
import { mkdir } from 'node:fs/promises';
test.use({serviceWorkers:'block'});
const sentFrames=new WeakMap<Page,any[]>(),receivedChats=new WeakMap<Page,Map<string,any>>();
test.beforeEach(async ({page}) => {const frames:any[]=[],chats=new Map<string,any>();sentFrames.set(page,frames);receivedChats.set(page,chats);page.on('websocket',socket=>{socket.on('framesent',frame=>{try{frames.push(JSON.parse(String(frame.payload)));}catch{}});socket.on('framereceived',frame=>{try{for(const chat of JSON.parse(String(frame.payload)).data?.chats??[])chats.set(chat.chat.id,chat);}catch{}});});await page.goto('/messages');await expect(page.getByRole('listbox',{name:'Conversations'})).toBeVisible();});
test('WEB-MSG-LIST row anatomy, avatars and wording',async({page},info)=>{
    const rows=page.locator('.conversation-row');
    await expect(rows.first()).toContainText('Maya Chen');
    await expect(rows.first().locator('.avatar')).toHaveCSS('width',info.project.use.viewport?.width===390?'52px':'40px');
    await expect(rows.first().locator('.avatar')).toContainText('MC');
    await expect(rows.first().locator('time')).toHaveText(new Intl.DateTimeFormat('en-US',{hour:'numeric',minute:'2-digit'}).format(new Date((await rows.first().locator('time').getAttribute('datetime'))!)));
    if(info.project.use.viewport?.width===390){const chevron=rows.first().locator('.conversation-chevron');await expect(chevron).toBeVisible();expect(await chevron.evaluate(element=>new DOMMatrix(getComputedStyle(element).transform).a)).toBe(-1);await page.evaluate(()=>document.documentElement.dir='rtl');expect(await chevron.evaluate(element=>new DOMMatrix(getComputedStyle(element).transform).a)).toBe(1);await page.evaluate(()=>document.documentElement.dir='ltr');}
    await expect(rows.filter({hasText:'Archived fixture'})).toHaveCount(0);
    await expect(page.locator('.conversation-row').filter({hasText:'Weekend Crew'}).locator('.avatar-member')).toHaveCount(3);
    for(const ratio of await rows.filter({hasText:'Weekend Crew'}).locator('.avatar-member').evaluateAll(elements=>elements.map(element=>parseFloat(getComputedStyle(element).fontSize)/element.getBoundingClientRect().width))){expect(ratio).toBeGreaterThan(.38);expect(ratio).toBeLessThan(.46);}
    await expect(page.getByText('Attachment: 2 Images',{exact:true})).toBeVisible();
    await expect(page.getByText('Loved “See you soon”',{exact:true})).toBeVisible();
    await expect(page.getByLabel('Notifications silenced')).toHaveCount(1);await expect(rows.filter({hasText:'Alex Morgan'}).getByLabel('Notifications silenced')).toBeVisible();
    const photo=rows.filter({hasText:'Sam Rivera'}).locator('img');await expect(photo).toBeVisible();await expect(photo).toHaveAttribute('src',/variant=thumb/);await expect.poll(()=>photo.evaluate((image:HTMLImageElement)=>image.complete&&image.naturalWidth===64)).toBe(true);
    const group=rows.filter({hasText:'Conversation 010'});await expect(group.locator('.conversation-preview')).toHaveText('Taylor Quinn named the conversation “Friday plans”');await expect(group.locator('img')).toBeVisible();await expect(group.locator('img')).toHaveClass(/avatar-image-ready/);await expect(group.locator('.avatar-member')).toHaveCount(0);
    await expect(rows.filter({hasText:'Conversation 011'}).locator('.conversation-preview')).toHaveText('You unsent a message');
    const emptyGroup=rows.filter({hasText:'Conversation 012'}).locator('.avatar');await expect(emptyGroup).toHaveText('C');await expect(emptyGroup).toHaveCSS('background-image',/linear-gradient/);
    await expect(rows.filter({hasText:'Conversation 013'}).locator('.avatar-member')).toHaveCount(0);
    const pair=rows.filter({hasText:'Conversation 014'}).locator('.avatar-member');await expect(pair).toHaveCount(2);const tops=await pair.evaluateAll(elements=>elements.map(element=>element.getBoundingClientRect().top));expect(tops[1]).toBeGreaterThan(tops[0]);
});
test('WEB-MSG-LIST relative dates, selection and window focus',async({page},info)=>{
    const rows=page.locator('.conversation-row');
    const listViewport=page.getByRole('listbox',{name:'Conversations'});
    await expect.poll(()=>['Conversation 005','Conversation 006','Conversation 007'].every(title=>[...receivedChats.get(page)!.values()].some(chat=>chat.title===title))).toBe(true);
    const ordered=[...receivedChats.get(page)!.values()].filter(chat=>chat.chat.data.is_archived!==true&&chat.pinned_index==null).sort((a,b)=>Date.parse(b.last_activity_at)-Date.parse(a.last_activity_at)||a.chat.id.localeCompare(b.chat.id));
    for(const [title,label] of [['Conversation 005',/^Yesterday$/],['Conversation 006',/^(Monday|Tuesday|Wednesday|Thursday|Friday|Saturday|Sunday)$/],['Conversation 007',/^\d{1,2}\/\d{1,2}\/\d{2}$/]] as const){
        const row=rows.filter({hasText:title}),index=ordered.findIndex(chat=>chat.title===title);expect(index).toBeGreaterThanOrEqual(0);
        await listViewport.evaluate((element,top)=>element.scrollTop=top,index*(info.project.use.viewport?.width===390?84:68));
        await expect(row.locator('time')).toHaveText(label);await listViewport.evaluate(element=>element.scrollTop=0);await expect(rows.first()).toContainText('Maya Chen');
    }
    await expect(rows.first().locator('.unread-dot')).toBeVisible();
    const target=rows.filter({hasText:'Weekend Crew'}); const id=await target.getAttribute('data-chat');
    await target.click(); await expect(page).toHaveURL(new RegExp('/messages/'+id+'$'));
    if(info.project.use.viewport?.width===1440){
        await expect(target.locator('..')).toHaveAttribute('aria-selected','true');
        await page.evaluate(()=>{Object.defineProperty(document,'hasFocus',{configurable:true,value:()=>true});window.dispatchEvent(new Event('focus'));});
        await expect(page.locator('.conversation-list')).not.toHaveClass(/window-inactive/);
        const fill=await target.evaluate(element=>getComputedStyle(element).backgroundColor);await expect(target.locator('.avatar-member').first()).toHaveCSS('border-top-color',fill);await mkdir('../lab/evidence/private/web',{recursive:true});await page.screenshot({path:`../lab/evidence/private/web/list-selected-focused-${info.project.name}.png`});
        await page.evaluate(()=>{Object.defineProperty(document,'hasFocus',{configurable:true,value:()=>false});window.dispatchEvent(new Event('blur'));});
        await expect(page.locator('.conversation-list')).toHaveClass(/window-inactive/);
        expect(await target.evaluate(element=>getComputedStyle(element).backgroundColor)).not.toBe(fill);await page.screenshot({path:`../lab/evidence/private/web/list-selected-inactive-${info.project.name}.png`});await rows.filter({hasText:'Sam Rivera'}).hover();await page.screenshot({path:`../lab/evidence/private/web/list-hover-${info.project.name}.png`});
    }
});
test('WEB-MSG-REORDER live rows spring to the top and unread dots animate once',async({page,request},info)=>{
    const rows=page.locator('.conversation-row'),id=await rows.filter({hasText:'Weekend Crew'}).getAttribute('data-chat');
    await page.evaluate(()=>{(window as any).__rowMotion=[];(window as any).__dotMotion=[];new MutationObserver(changes=>{for(const change of changes){const row=change.target as HTMLElement;if(row.matches('.conversation-row')&&row.style.transform)(window as any).__rowMotion.push({chat:row.dataset.chat,transform:row.style.transform});if(row.matches('.unread-dot')&&row.style.transform)(window as any).__dotMotion.push({chat:(row.closest('.conversation-row') as HTMLElement)?.dataset.chat,transform:row.style.transform});}}).observe(document.querySelector('.conversation-list')!,{subtree:true,attributes:true,attributeFilter:['style']});});
    const mayaID=await rows.first().getAttribute('data-chat'),viewport=page.getByRole('listbox',{name:'Conversations'});
    await viewport.evaluate(element=>element.scrollTop=1000);await expect(page.locator(`[data-chat="${mayaID}"]`)).toHaveCount(0);await viewport.evaluate(element=>element.scrollTop=0);await expect(rows.first()).toContainText('Maya Chen');
    expect(await page.evaluate(id=>(window as any).__dotMotion.some((entry:any)=>entry.chat===id),mayaID)).toBe(false);
    await control(request,'/list-fixture?update=1');
    await expect(rows.first()).toContainText('Weekend Crew');
    await expect(rows.first()).toContainText('The plan is ready');
    await expect.poll(()=>page.evaluate(()=>(window as any).__rowMotion.length)).toBeGreaterThan(0);
    await expect.poll(()=>page.evaluate(id=>(window as any).__dotMotion.some((entry:any)=>entry.chat===id),id)).toBe(true);
    await expect(page.getByRole('button',{name:'New Message',exact:true})).toBeVisible();
    await expect.poll(()=>rows.evaluateAll(elements=>elements.every(element=>!(element as HTMLElement).style.transform))).toBe(true);
    await expect.poll(()=>rows.first().locator('.unread-dot').evaluate(element=>element.getBoundingClientRect().width)).toBe(8);
    await mkdir('../lab/evidence/private/web',{recursive:true});
    await page.screenshot({path:`../lab/evidence/private/web/list-fixture-${info.project.name}.png`});

});

test('WEB-MSG-AVATAR-READY failed and empty photos retain a visible monogram',async({page})=>{
    const rows=page.locator('.conversation-row'),photo=rows.filter({hasText:'Sam Rivera'}).locator('img');await expect(photo).toBeVisible();const photoSource=await photo.getAttribute('src');let photoHits=0;
    await page.route('**/web/v1/blob/**',route=>{if(route.request().url().endsWith(photoSource!))photoHits++;return route.fulfill({status:404,body:'not found'});});await page.reload();await expect.poll(()=>photoHits).toBeGreaterThan(0);
    await expect(rows.filter({hasText:'Sam Rivera'}).locator('.avatar')).toHaveText('SR');await expect(rows.filter({hasText:'Sam Rivera'}).locator('img')).toHaveCount(0);
    await page.unroute('**/web/v1/blob/**');photoHits=0;await page.route('**/web/v1/blob/**',route=>{if(route.request().url().endsWith(photoSource!))photoHits++;return route.fulfill({status:200,contentType:'image/png',body:Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAAC0lEQVR4nGP4DwQACfsD/fteaysAAAAASUVORK5CYII=','base64')});});await page.reload();await expect.poll(()=>photoHits).toBeGreaterThan(0);
    await expect(rows.filter({hasText:'Sam Rivera'}).locator('.avatar')).toHaveText('SR');await expect(rows.filter({hasText:'Sam Rivera'}).locator('img')).toHaveCount(0);await page.unroute('**/web/v1/blob/**');await page.reload();

});
test('WEB-MSG-LOADING coherent cached rows paint before an offline connection without skeletons',async({page})=>{
    const viewport=page.getByRole('listbox',{name:'Conversations'});
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
    const pinFrames=sentFrames.get(page)!;await control(request,'/list-pins');
    const pins=page.getByRole('group',{name:'Pinned conversations'});
    await expect(pins.getByRole('button')).toHaveCount(3);
    await expect(pins.getByRole('button',{name:'Maya Chen',exact:true}).locator('.pin-preview')).toContainText('Are we still on for tomorrow?');await expect(pins.getByRole('button',{name:'Weekend Crew',exact:true}).locator('.pin-preview')).toHaveCount(0);const sidebar=(await page.locator('.sidebar').boundingBox())!;for(const bubble of await pins.locator('.pin-preview').all()){const box=(await bubble.boundingBox())!;expect(box.x).toBeGreaterThanOrEqual(sidebar.x);expect(box.x+box.width).toBeLessThanOrEqual(sidebar.x+sidebar.width);}
    await control(request,'/list-fixture?update=1');await expect(pins.getByRole('button',{name:'Weekend Crew',exact:true}).locator('.pin-preview')).toHaveText('The plan is ready');expect(pinFrames.some(frame=>frame.t==='sub'&&frame.view==='chats')).toBe(true);expect(pinFrames.some(frame=>frame.t==='sub'&&frame.view==='thread')).toBe(false);await mkdir('../lab/evidence/private/web',{recursive:true});await page.screenshot({path:`../lab/evidence/private/web/list-pins-${info.project.name}.png`});
    if(info.project.use.viewport?.width===390){await pins.getByRole('button',{name:'Maya Chen',exact:true}).click();await expect(page).toHaveURL(/messages\/cht_/);return;}
    const modifier=await page.evaluate(()=>/Mac|iPhone|iPad/.test(navigator.platform)?'Meta':'Control');
    await page.keyboard.press(modifier+'+2');await expect(pins.getByRole('button',{name:'Weekend Crew'})).toHaveAttribute('aria-pressed','true');
    await page.keyboard.press(modifier+'+1');await expect(pins.getByRole('button',{name:'Maya Chen'})).toHaveAttribute('aria-pressed','true');
});
test('WEB-MSG-KEYBOARD-LIST arrows, chat cycling and New Message',async({page},info)=>{
    test.skip(info.project.use.viewport?.width===390,'Desktop keyboard shortcuts');
    const modifier=await page.evaluate(()=>/Mac|iPhone|iPad/.test(navigator.platform)?'Meta':'Control');
    await page.getByRole('listbox',{name:'Conversations'}).focus();
    await page.keyboard.press('ArrowDown');await expect(page.locator('.conversation-row').first().locator('..')).toHaveAttribute('aria-selected','true');
    await page.keyboard.press('ArrowDown');await expect(page.locator('.conversation-row').nth(1).locator('..')).toHaveAttribute('aria-selected','true');
    await page.keyboard.press('ArrowUp');await expect(page.locator('.conversation-row').first().locator('..')).toHaveAttribute('aria-selected','true');
    await page.keyboard.press('Control+Tab');await expect(page.locator('.conversation-row').nth(1).locator('..')).toHaveAttribute('aria-selected','true');
    await page.keyboard.press('Control+Shift+Tab');await expect(page.locator('.conversation-row').first().locator('..')).toHaveAttribute('aria-selected','true');
    await page.keyboard.press(modifier+'+n');await expect(page).toHaveURL(/messages\/new$/);await page.goBack();await page.getByRole('button',{name:'New Message',exact:true}).click();await expect(page).toHaveURL(/messages\/new$/);
});

test.describe('Public conversation loading',()=>{
    test.use({listKind:'public'});
test('WEB-MSG-LOADING uncached loading waits before skeletons and clears on live data',async({page})=>{
    let holding=true;const pending:(()=>void)[]=[];
    await page.routeWebSocket('**/web/v1/live',route=>{const server=route.connectToServer();route.onMessage(message=>server.send(message));server.onMessage(message=>{if(holding)pending.push(()=>route.send(message));else route.send(message);});});
    await page.addInitScript(()=>{const observer=new MutationObserver(()=>{if(!document.querySelector('.conversation-list'))return;observer.disconnect();setTimeout(()=>(window as any).__listEarlySkeleton=Boolean(document.querySelector('.list-skeleton')),100);});observer.observe(document,{subtree:true,childList:true});});
    await page.goto('/messages');await expect.poll(()=>page.evaluate(()=>(window as any).__listEarlySkeleton)).toBe(false);await expect(page.getByLabel('Loading conversations')).toBeVisible();
    holding=false;for(const send of pending.splice(0))send();
    await expect(page.getByRole('listbox',{name:'Conversations'})).toBeVisible();await expect(page.locator('.list-skeleton')).toHaveCount(0);
});

});

test('WEB-MSG-LIST unread title, favicon and app badge survive Settings and clear on sign-out',async({page})=>{
    await page.addInitScript(()=>{(window as any).__badges=[];Object.defineProperty(navigator,'setAppBadge',{configurable:true,value:async(count:number)=>(window as any).__badges.push(count)});Object.defineProperty(navigator,'clearAppBadge',{configurable:true,value:async()=>(window as any).__badges.push(0)});});
    await page.reload();await expect.poll(async()=>Number((await page.title()).match(/^\((\d+)\) Messages$/)?.[1]??0)).toBeGreaterThanOrEqual(270);const title=await page.title(),count=Number(title.match(/\d+/)![0]);await expect.poll(()=>page.evaluate(()=>(window as any).__badges.at(-1))).toBe(count);await expect(page.locator('link[rel="icon"]')).toHaveAttribute('href',/^data:image\/svg\+xml,/);
    await page.getByRole('button',{name:'Switch app',exact:true}).click();await page.getByRole('button',{name:'Settings',exact:true}).click();await expect(page).toHaveURL(/settings\/devices$/);await expect(page).toHaveTitle(title);await expect.poll(()=>page.evaluate(()=>(window as any).__badges.at(-1))).toBe(count);
    await page.getByRole('button',{name:'Sign out',exact:true}).click();await expect(page.getByRole('heading',{name:'Sign in with a passkey',exact:true})).toBeVisible();await expect(page).toHaveTitle('Messages');await expect.poll(()=>page.evaluate(()=>(window as any).__badges.at(-1))).toBe(0);await expect(page.locator('link[rel="icon"]')).toHaveAttribute('href','/icon.svg');
});

test('WEB-MSG-LIST glass chrome keeps the last conversation reachable',async({page},info)=>{
    await expect(page.getByRole('searchbox',{name:'Search',exact:true})).toBeVisible();await expect(page.getByRole('button',{name:'New Message',exact:true})).toBeVisible();await expect(page.getByRole('button',{name:'Details',exact:true})).toHaveCount(0);
    const viewport=page.getByRole('listbox',{name:'Conversations'});await expect.poll(async()=>Number(await viewport.locator('[aria-setsize]').first().getAttribute('aria-setsize'))).toBeGreaterThanOrEqual(540);await viewport.evaluate(element=>element.scrollTop=element.scrollHeight);
    await expect.poll(async()=>{const last=viewport.locator('[aria-posinset]').last();return await last.getAttribute('aria-posinset')===await last.getAttribute('aria-setsize');}).toBe(true);
    const last=(await page.locator('.conversation-row').last().boundingBox())!,chrome=(await page.locator(info.project.use.viewport?.width===390?'.phone-search':'.sidebar-footer').boundingBox())!;expect(last.y+last.height).toBeLessThanOrEqual(chrome.y+1);
});

test('WEB-MSG-AVATAR-READY delayed handles never publish provisional initials or move the monogram box',async({page})=>{
    await page.emulateMedia({reducedMotion:'no-preference'});await page.goto('about:blank');
    let release!:()=>void;const held=new Promise<void>(resolve=>release=resolve);
    await page.route('**/web/v1/handles',async route=>{const response=await route.fetch();await held;await route.fulfill({response});});
    await page.addInitScript(()=>{(window as any).__avatarText=[];new MutationObserver(records=>{const row=[...document.querySelectorAll('.conversation-row')].find(element=>element.querySelector('.conversation-name')?.textContent==='Demo Busy Workshop'),avatar=row?.querySelector('.avatar');if(!avatar)return;for(const record of records){const element=record.target instanceof Element?record.target:record.target.parentElement;if(element&&avatar.contains(element)){if(record.oldValue)(window as any).__avatarText.push(record.oldValue);for(const node of record.addedNodes)if(node.textContent)(window as any).__avatarText.push(node.textContent);}}if(avatar.textContent)(window as any).__avatarText.push(avatar.textContent);}).observe(document,{subtree:true,childList:true,characterData:true,characterDataOldValue:true});});
    try{
        await page.goto('/messages');await page.getByRole('searchbox',{name:'Search',exact:true}).fill('Demo Busy Workshop');
        const avatar=page.locator('.conversation-row').filter({hasText:'Demo Busy Workshop'}).locator('.avatar'),monogram=avatar.locator('.avatar-monogram');
        await expect(avatar.locator('.avatar-person')).toBeVisible();await expect(monogram).toHaveText('');await expect(monogram).toHaveCSS('opacity','0');
        const before=await monogram.evaluate(element=>{const box=element.getBoundingClientRect();return {x:box.x,y:box.y,width:box.width,height:box.height};});expect(before.width).toBeGreaterThan(0);expect(before.height).toBeGreaterThan(0);
        await page.waitForTimeout(200);await expect(monogram).toHaveText('');release();
        await expect(monogram).toHaveText('DE');await expect(monogram).toHaveCSS('opacity','1');
        const observed=await page.evaluate(()=>(window as any).__avatarText as string[]);expect(observed.length).toBeGreaterThan(0);expect(observed.every(value=>value==='DE')).toBe(true);
        const boxes=await monogram.evaluate(async element=>{const values=[];for(let index=0;index<12;index++){await new Promise<void>(resolve=>requestAnimationFrame(()=>resolve()));const box=element.getBoundingClientRect();values.push({x:box.x,y:box.y,width:box.width,height:box.height});}return values;});
        for(const box of boxes)expect(box).toEqual(before);
        const motion=await monogram.evaluate(element=>({property:getComputedStyle(element).transitionProperty,duration:getComputedStyle(element).transitionDuration}));expect(motion.property).toBe('opacity');expect(parseFloat(motion.duration)).toBeGreaterThan(0);expect(parseFloat(motion.duration)).toBeLessThanOrEqual(.15);
        await page.emulateMedia({reducedMotion:'reduce'});await expect(monogram).toHaveCSS('transition-duration','0s');
    }finally{release();await page.unrouteAll({behavior:'wait'});}
});
