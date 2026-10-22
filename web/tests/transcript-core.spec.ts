import { test, expect, type Page, type APIRequestContext } from '@playwright/test';
import { mkdir, writeFile } from 'node:fs/promises';
import { prepareSession, control } from './session';
test.use({deviceScaleFactor:2});
type Fixtures=Record<string,string>;
const native=(index:number)=>'demo-message-00-'+String(index).padStart(6,'0');
async function fixture(page:Page,request:APIRequestContext,index:number){
    await prepareSession(page);
    const ids:Fixtures=await control(request,'/demo-fixtures');
    await page.clock.setFixedTime(new Date('2026-10-20T16:00:00Z'));
    await jump(page,ids,index);
    return ids;
}
async function jump(page:Page,ids:Fixtures,index:number,chat='message-chat'){
    await page.goto('/messages/'+encodeURIComponent(ids[chat])+'/'+encodeURIComponent(ids[native(index)]));
    await expect(page.locator('[data-message="'+ids[native(index)]+'"]')).toBeVisible();
    await expect.poll(()=>page.locator('.transcript').evaluate(element=>getComputedStyle(element).transform)).toBe('none');
}
const row=(page:Page,ids:Fixtures,index:number)=>page.locator('[data-message="'+ids[native(index)]+'"]');
async function capture(page:Page,name:string,project:string){await mkdir('../lab/evidence/private/web',{recursive:true});await page.screenshot({path:`../lab/evidence/private/web/transcript-${name}-${project}.png`});}
test('WEB-MSG-THREAD-CORE transcript joins runs, preserves geometry and renders service colors',async({page,request},info)=>{
    const ids=await fixture(page,request,58);
    for(let index=58;index<=65;index++)await expect(row(page,ids,index).locator('.bubble-tail')).toHaveCount(index===61||index===65?1:0);
    const tails=await page.locator('.message-row .bubble-tail path').evaluateAll(elements=>elements.map(element=>{const path=element.getBoundingClientRect(),row=element.closest('.message-row')!.getBoundingClientRect(),viewport=element.closest('.virtual-viewport')!.getBoundingClientRect();return{message:element.closest('.message-row')!.getAttribute('data-message'),pathBottom:path.bottom,rowBottom:row.bottom,viewportTop:viewport.top,overflow:path.bottom-row.bottom};}));expect(tails.length).toBeGreaterThanOrEqual(2);await mkdir('../lab/evidence/private/web',{recursive:true});await writeFile(`../lab/evidence/private/web/transcript-tail-baseline-${info.project.name}.json`,JSON.stringify(tails,null,2)+'\n');for(const tail of tails)expect(tail.overflow,tail.message??'tail').toBeLessThanOrEqual(.01);
    for(const index of [61,65]){const silhouette=await row(page,ids,index).locator('.bubble').evaluate(element=>{const shape=element.querySelector('path')!.getBBox();return{width:element.clientWidth,height:element.clientHeight,shape:{width:shape.width,height:shape.height}};});expect(silhouette.shape.width).toBeCloseTo(silhouette.width+5,1);expect(silhouette.shape.height).toBeCloseTo(silhouette.height,3);}
    await expect(row(page,ids,58).locator('.sender-name')).toHaveText('Demo Beryl01 Example');
    await expect(row(page,ids,59).locator('.sender-name')).toHaveCount(0);
    await expect(row(page,ids,61).locator('.sender-avatar')).toHaveCount(1);
    await expect(row(page,ids,58).locator('.bubble')).toHaveCSS('background-color',info.project.name.includes('-dark-')?'rgb(38, 38, 40)':'rgb(233, 233, 235)');
    await expect(row(page,ids,62).locator('.bubble')).toHaveCSS('background-image',/linear-gradient/);
    const bounds=await row(page,ids,58).locator('.bubble').boundingBox();expect(bounds!.width).toBeLessThanOrEqual(info.project.name.endsWith('-phone')?390*.7:480);
    await capture(page,'core',info.project.name);
    await jump(page,ids,1);await expect(row(page,ids,1).locator('.emoji-only')).toHaveCount(1);await expect(row(page,ids,1).locator('.emoji-only')).toHaveCSS('background-color','rgba(0, 0, 0, 0)');await expect(row(page,ids,1).locator('.bubble-tail')).toHaveCount(0);
    await jump(page,ids,0);const header=row(page,ids,0).locator('[data-date-header]');await expect(header).toContainText('2024');await expect(header.locator('strong')).toHaveCSS('font-weight','700');await expect(header).toContainText(/ at \d{1,2}:\d{2} [AP]M$/u);
    const center=await header.evaluate(element=>{const rect=element.getBoundingClientRect(),viewport=element.closest('.virtual-viewport')!,bounds=viewport.getBoundingClientRect();return rect.left+rect.width/2-bounds.left-viewport.clientWidth/2;});expect(Math.abs(center)).toBeLessThan(1);
    await jump(page,ids,32);
    const media=row(page,ids,32).locator('.media-reserved');const before=await media.boundingBox();expect(before!.width/before!.height).toBeCloseTo(240/400,2);
    await expect(media.locator('img')).toBeVisible();await expect.poll(()=>media.locator('img').evaluate((element:HTMLImageElement)=>element.complete&&element.naturalWidth>0)).toBe(true);expect(await media.boundingBox()).toEqual(before);
    await expect(row(page,ids,32).locator('.bubble')).toHaveCount(2);
    if(info.project.name.endsWith('-phone')){const viewport=page.viewportSize()!;await page.setViewportSize({...viewport,height:500});await expect.poll(async()=>Math.round((await media.boundingBox())!.height)).toBe(300);await expect(media.locator('img')).toHaveCSS('object-fit','cover');await capture(page,'portrait-cap',info.project.name);await page.setViewportSize(viewport);await expect.poll(async()=>Math.round((await media.boundingBox())!.height)).toBe(400);}
    await jump(page,ids,36);const alpha=row(page,ids,36).locator('.media-reserved');await expect(alpha).toHaveCSS('background-color','rgba(0, 0, 0, 0)');await expect(alpha).toHaveCSS('border-top-width','0px');await expect(alpha).toHaveCSS('border-top-left-radius','0px');await expect(alpha).toHaveCSS('overflow','visible');await expect(alpha.locator('img')).toHaveCSS('object-fit','contain');await page.getByRole('listbox',{name:'Messages transcript'}).focus();await expect(alpha).toHaveCSS('border-top-left-radius','0px');await page.getByRole('listbox',{name:'Messages transcript'}).evaluate(element=>(element as HTMLElement).blur());await expect(alpha.locator('.media-image')).toHaveAttribute('data-media-state','ready');await capture(page,'alpha',info.project.name);
    const alphaBounds=await alpha.boundingBox();expect(alphaBounds!.width).toBeLessThanOrEqual(256);expect(alphaBounds!.height).toBeLessThanOrEqual(256);
    await jump(page,ids,38);const video=row(page,ids,38).locator('.media-reserved'),videoBounds=await video.boundingBox(),phone=info.project.name.endsWith('-phone');
    expect(videoBounds!.width).toBeGreaterThanOrEqual(120);expect(videoBounds!.height).toBeGreaterThanOrEqual(120);
    expect(videoBounds!.width).toBeLessThanOrEqual(phone?390*.7:480);expect(videoBounds!.height).toBeLessThanOrEqual(phone?page.viewportSize()!.height*.6:360);
    await capture(page,'video-minimum',info.project.name);
    await jump(page,ids,129);await expect(row(page,ids,129).locator('.bubble')).toHaveCount(1);
    await jump(page,ids,111);await expect(row(page,ids,111).locator('.bubble')).toHaveCount(1);
    await jump(page,ids,198);await expect(row(page,ids,198).locator('.receipt')).toContainText('Read');await expect(page.locator('.receipt')).toHaveCount(1);
    for(const chat of ['demo-chat-01','demo-chat-02']){
        await page.goto('/messages/'+encodeURIComponent(ids[chat]));const sent=page.locator('.bubble.sent.service-green').first();await expect(sent).toBeVisible();await expect(sent.locator('.bubble-tail path')).toHaveCSS('fill',info.project.name.includes('-dark-')?'rgb(48, 209, 88)':'rgb(52, 199, 89)');
    }
});
test('WEB-MSG-THREAD-CORE transcript detects links and limits a mention to its UTF-16 range',async({page,request})=>{
    const ids=await fixture(page,request,4);await expect(row(page,ids,4).getByRole('link')).toHaveCount(2);await expect(row(page,ids,4).getByRole('link').first()).toHaveAttribute('href','https://example.com/demo?view=garden');
    await jump(page,ids,5);const mention=row(page,ids,5).getByRole('button',{name:'Demo Aster00',exact:true});await expect(mention).toBeVisible();await expect(mention).toHaveCSS('font-weight','600');await mention.click();await expect(page.getByRole('dialog',{name:'Contact'})).toContainText('demo.person00@example.com');
    await jump(page,ids,113);const formatted=row(page,ids,113);await expect(formatted.getByText('Bold',{exact:true})).toHaveCSS('font-weight','700');await expect(formatted.getByText('Bold',{exact:true})).toHaveCSS('font-style','italic');await expect(formatted.getByText('paper',{exact:true})).toHaveCSS('text-decoration-line',/underline/);await expect(formatted.getByText('paper',{exact:true})).toHaveCSS('text-decoration-line',/line-through/);
    await jump(page,ids,123);await expect(row(page,ids,123).getByText('Synthetic studio update',{exact:true})).toHaveCSS('font-weight','700');
});
test('WEB-MSG-THREAD-CORE transcript shows edit, unsent, failure and group-event lines',async({page,request})=>{
    const ids=await fixture(page,request,11);
    await expect(row(page,ids,11).getByRole('button',{name:'Edited',exact:true})).toBeVisible();
    await expect(row(page,ids,12).locator('.system-line')).toHaveText(/unsent a message$/);
    await expect(row(page,ids,13).locator('.failure-line')).toHaveText('! Not Delivered');
    await expect(row(page,ids,13).locator('.receipt')).toHaveCount(0);
    await jump(page,ids,26);
    for(const [index,pattern] of [[26,/named the conversation “Demo Lantern Club”$/],[27,/added Demo /],[28,/removed Demo /],[29,/left the conversation$/],[30,/changed the group photo$/]] as const){await expect(row(page,ids,index).locator('.system-line')).toHaveText(pattern);await expect(row(page,ids,index).locator('.bubble')).toHaveCount(0);}
});
test('WEB-MSG-THREAD-CORE transcript opens at unread and bounds its paged window',async({page,request})=>{
    await prepareSession(page);const ids:Fixtures=await control(request,'/demo-fixtures');
    const anchors:string[]=[];
    page.on('websocket',socket=>socket.on('framereceived',event=>{const frame=JSON.parse(String(event.payload));if(frame.t==='snap'&&frame.data?.unread_anchor)anchors.push(frame.data.unread_anchor);}));
    await page.goto('/messages/'+encodeURIComponent(ids['message-chat']));
    await expect.poll(()=>anchors.length).toBeGreaterThan(0);
    await expect(page.locator('[data-message="'+anchors[0]+'"]')).toBeVisible();
    await expect(page.getByRole('listbox',{name:'Messages transcript'})).toBeVisible();
    await page.goto('/messages/'+encodeURIComponent(ids['demo-chat-01']));
    await expect(page.locator('.bubble.sent.service-green').first()).toBeVisible();
    const sizes=await page.locator('.messages-transcript').evaluate(element=>({window:Number((element as HTMLElement).dataset.windowSize),rendered:element.querySelectorAll('[data-message]').length}));
    expect(sizes.window).toBeGreaterThan(0);expect(sizes.window).toBeLessThanOrEqual(500);expect(sizes.rendered).toBeLessThan(100);
});
test('WEB-MSG-THREAD-CORE transcript keeps a visible bubble fixed when paging forward',async({page,request})=>{
    await prepareSession(page);const ids:Fixtures=await control(request,'/demo-fixtures');
    await page.goto('/messages/'+encodeURIComponent(ids['demo-chat-01']));
    const window=page.locator('.messages-transcript'),viewport=page.getByRole('listbox',{name:'Messages transcript'});
    await expect(viewport).toBeVisible();await expect(window).toHaveAttribute('aria-busy','false');
    for(let index=0;index<2;index++){
        const anchor=await window.getAttribute('data-window-anchor');
        await viewport.evaluate(element=>element.scrollTop=0);
        await expect(window).not.toHaveAttribute('data-window-anchor',anchor!);
        await expect(window).toHaveAttribute('aria-busy','false');
    }
    const last=await window.getAttribute('data-window-end'),anchor=await window.getAttribute('data-window-anchor');
    await viewport.evaluate(element=>element.scrollTop=element.scrollHeight);
    await expect(window).not.toHaveAttribute('data-window-anchor',anchor!);
    await expect(window).toHaveAttribute('aria-busy','false');
    const bubble=page.locator('[data-message="'+last+'"]');await expect(bubble).toBeVisible();
    const difference=await bubble.evaluate(element=>element.getBoundingClientRect().bottom-element.closest('.virtual-viewport')!.getBoundingClientRect().bottom);
    expect(Math.abs(difference)).toBeLessThanOrEqual(1);
    expect(Number(await window.getAttribute('data-window-size'))).toBeLessThanOrEqual(500);
});
