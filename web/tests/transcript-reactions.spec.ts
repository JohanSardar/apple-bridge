import { test, expect } from '@playwright/test';
import { mkdir } from 'node:fs/promises';
import { prepareSession, control } from './session';
test.use({deviceScaleFactor:2});
test('WEB-MSG-REACTIONS transcript badges identify each reactor and preserve bubble geometry',async({page,request},info)=>{
    let capturing=false,screenshotPolicyErrors=0;const policyErrors:string[]=[];page.on('console',message=>{if(message.type()==='error'&&message.text().includes('Content Security Policy')){if(capturing)screenshotPolicyErrors++;else policyErrors.push(message.text());}});
    const capture=async(path:string)=>{capturing=true;try{await page.screenshot({caret:'initial',path});}finally{capturing=false;}};
    await prepareSession(page);
    const ids:Record<string,string>=await control(request,'/demo-fixtures');
    const native=(index:number)=>'demo-message-00-'+String(index).padStart(6,'0');
    const row=(index:number)=>page.locator('[data-message="'+ids[native(index)]+'"]');
    const jump=async(index:number)=>{await page.goto('/messages/'+encodeURIComponent(ids['message-chat'])+'/'+encodeURIComponent(ids[native(index)]));await expect(row(index).locator('..')).toHaveAttribute('aria-selected','true');await expect(row(index)).toBeInViewport();await expect.poll(()=>page.locator('.transcript').evaluate(element=>getComputedStyle(element).transform)).toBe('none');};
    await jump(50);
    const symbols=['❤️','👍','👎','😂','‼️','❓','🪁'];
    for(let index=50;index<=56;index++){
        if(index>50)await jump(index);
        const badges=row(index).getByRole('button',{name:/reacted/});
        await expect(badges).toHaveCount(1);
        await expect(badges.first().locator('.reaction-count')).toHaveText('3');
        await expect(row(index).locator('.reaction-badge.own-reaction')).toHaveCSS('background-color',info.project.name.includes('-dark-')?'rgb(0, 145, 255)':'rgb(0, 136, 255)');
        for(const badge of await badges.all())await expect(badge).toContainText(symbols[index-50]);
        const geometry=await row(index).evaluate(element=>{
            const bubble=element.querySelector('.bubble')!.getBoundingClientRect(),range=document.createRange();range.selectNodeContents(element.querySelector('.message-text')!);const text=range.getBoundingClientRect(),badges=Array.from(element.querySelectorAll('.reaction-badge'),badge=>badge.getBoundingClientRect());
            return{viewportTop:element.closest('.virtual-viewport')!.getBoundingClientRect().top,textTop:text.top,sent:element.classList.contains('outgoing'),bubble:{left:bubble.left,right:bubble.right,top:bubble.top,bottom:bubble.bottom},badges:badges.map(value=>({left:value.left,right:value.right,top:value.top,bottom:value.bottom}))};
        });
        expect(geometry.badges[0].top).toBeLessThan(geometry.bubble.top);
        for(const badge of geometry.badges)expect(badge.top).toBeGreaterThanOrEqual(geometry.viewportTop-.25);
        expect(geometry.badges[0].bottom).toBeGreaterThan(geometry.bubble.top);
        for(const badge of geometry.badges)expect(badge.bottom).toBeLessThanOrEqual(geometry.textTop+.25);
        const center=(geometry.badges[0].left+geometry.badges[0].right)/2,bubbleCenter=(geometry.bubble.left+geometry.bubble.right)/2;
        expect(geometry.sent?center<bubbleCenter:center>bubbleCenter).toBe(true);
    }
    await jump(57);await expect(row(57).locator('.reaction-badge')).toHaveCount(0);
    await jump(50);
    const bubble=row(50).locator('.bubble'),before=await bubble.boundingBox();
    await row(50).getByRole('button',{name:/reacted/}).first().click();
    await expect(row(50).locator('.bubble')).toHaveCSS('outline-style','none');
    const dialog=page.getByRole('dialog',{name:'Reactions',exact:true});
    await expect(dialog).toBeVisible();
    expect(await dialog.evaluate(element=>getComputedStyle(element,'::backdrop').getPropertyValue('backdrop-filter'))).toBe(info.project.name.endsWith('phone')?'blur(3px)':'blur(5px)');
    expect(await dialog.evaluate(element=>getComputedStyle(element,'::backdrop').backgroundColor)).toBe('rgba(0, 0, 0, 0.3)');
    const sheet=await dialog.boundingBox();if(info.project.name.endsWith('phone'))expect(sheet!.width).toBe(page.viewportSize()!.width-24);else expect(sheet!.width).toBeLessThanOrEqual(360);
    if(!info.project.name.endsWith('phone')){const anchor=await bubble.boundingBox();expect(sheet!.y+sheet!.height<=anchor!.y||sheet!.y>=anchor!.y+anchor!.height).toBe(true);expect(await dialog.evaluate(element=>getComputedStyle(element,'::backdrop').clipPath)).not.toBe('none');const mask=await dialog.evaluate(element=>{const d=getComputedStyle(element,'::backdrop').clipPath.match(/path\(evenodd,\s*"(.*)"\)/)![1],bubble=document.querySelector('.reaction-anchor-raised')!.getBoundingClientRect(),context=document.createElement('canvas').getContext('2d')!,path=new Path2D(d);return{inside:context.isPointInPath(path,(bubble.left+bubble.right)/2,(bubble.top+bubble.bottom)/2,'evenodd'),outside:context.isPointInPath(path,bubble.left-2,(bubble.top+bubble.bottom)/2,'evenodd')};});expect(mask).toEqual({inside:false,outside:true});await expect(dialog.locator('[data-reaction-arrow]')).toBeVisible();}
    await expect(dialog.getByRole('button',{name:'Close Reactions'})).toHaveCSS('width','30px');
    await expect(dialog.getByRole('button',{name:'Close Reactions'})).toHaveCSS('outline-style','none');
    await expect(dialog.getByRole('heading',{name:'Reactions'})).toHaveCSS('outline-style','none');
    for(const name of ['You','Demo Beryl01 Example','Demo Cobalt02 Example'])await expect(dialog.getByText(name,{exact:true})).toBeVisible();
    await mkdir('../lab/evidence/private/web',{recursive:true});
    await capture(`../lab/evidence/private/web/transcript-who-reacted-${info.project.name}.png`);
    await page.keyboard.press('Escape');await expect(dialog).not.toBeVisible();
    expect(await bubble.boundingBox()).toEqual(before);
    expect(policyErrors).toEqual([]);
    expect(await page.evaluate(()=>Array.from(document.styleSheets).flatMap(sheet=>Array.from(sheet.cssRules)).filter(rule=>rule.cssText.includes('[data-reaction-popover=')).length)).toBe(0);
    await mkdir('../lab/evidence/private/web',{recursive:true});
    await capture(`../lab/evidence/private/web/transcript-reactions-${info.project.name}.png`);
    if(!info.project.name.endsWith('phone')){
        await bubble.evaluate(element=>{const viewport=element.closest('.virtual-viewport')!;viewport.scrollTop-=viewport.getBoundingClientRect().bottom-element.getBoundingClientRect().bottom-96;});
        await expect.poll(()=>bubble.evaluate(element=>element.closest('.virtual-viewport')!.getBoundingClientRect().bottom-element.getBoundingClientRect().bottom)).toBeLessThan(110);
        await row(50).getByRole('button',{name:/reacted/}).first().click();await expect(dialog).toHaveAttribute('data-side','above');
        const above=await dialog.boundingBox(),anchor=await bubble.boundingBox();expect(above!.y+above!.height).toBeLessThan(anchor!.y);
        await page.keyboard.press('Escape');await expect(dialog).not.toBeVisible();expect(policyErrors).toEqual([]);
    }
    await info.attach('screenshot-policy-errors',{body:JSON.stringify({product_errors:policyErrors,screenshot_policy_errors:screenshotPolicyErrors,source:'Playwright 1.64.1 inPagePrepareForScreenshots syncAnimations injects body {} even with caret initial; raw console entries remain in trace'}),contentType:'application/json'});
});
test('WEB-MSG-REACTIONS transcript keeps emoji and sticker reactions on their own message part',async({page,request})=>{
    await prepareSession(page);
    const ids:Record<string,string>=await control(request,'/demo-fixtures');
    const fixture:{message:string}=await control(request,'/reaction-fixture');
    await page.goto('/messages/'+encodeURIComponent(ids['message-chat'])+'/'+encodeURIComponent(fixture.message));
    const row=page.locator('[data-message="'+fixture.message+'"]');
    const part=(index:number)=>row.locator('.bubble-slot').filter({has:page.locator('[data-part="'+index+'"]')});
    await expect(part(0).getByRole('button',{name:/reacted/})).toHaveCount(1);
    await expect(part(1).getByRole('button',{name:/reacted/})).toHaveCount(3);
    await expect(part(1).locator('.reaction-badge:not(.reaction-overflow)')).toHaveText(['👍','😂','▧']);
    const overflow=row.getByRole('button',{name:'See all 5 reactions'});await expect(overflow).toHaveText('+2');
    await overflow.click();
    const dialog=page.getByRole('dialog',{name:'Reactions',exact:true});
    await expect(dialog).toContainText('Demo Beryl01 Example');
    await expect(dialog).toContainText('🪁');await expect(dialog.locator('[data-sticker-placeholder]')).toBeVisible();await expect(dialog.locator('li')).toHaveCount(5);
    await expect(dialog).not.toContainText('❤️');
});
