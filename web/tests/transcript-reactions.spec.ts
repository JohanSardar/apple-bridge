import { test, expect } from '@playwright/test';
import { mkdir } from 'node:fs/promises';
import { prepareSession, control } from './session';
test.use({deviceScaleFactor:2});
test('WEB-MSG-REACTIONS transcript badges identify each reactor and preserve bubble geometry',async({page,request},info)=>{
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
        await expect(badges).toHaveCount(3);
        await expect(row(index).getByRole('button',{name:/^You reacted/})).toHaveCSS('background-color',info.project.name.includes('-dark-')?'rgb(0, 145, 255)':'rgb(0, 136, 255)');
        for(const badge of await badges.all())await expect(badge).toContainText(symbols[index-50]);
        const geometry=await row(index).evaluate(element=>{
            const bubble=element.querySelector('.bubble')!.getBoundingClientRect(),badges=Array.from(element.querySelectorAll('.reaction-badge'),badge=>badge.getBoundingClientRect());
            return{sent:element.classList.contains('outgoing'),bubble:{left:bubble.left,right:bubble.right,top:bubble.top,bottom:bubble.bottom},badges:badges.map(value=>({left:value.left,right:value.right,top:value.top,bottom:value.bottom}))};
        });
        expect(geometry.badges[0].top).toBeLessThan(geometry.bubble.top);
        expect(geometry.badges[0].bottom).toBeGreaterThan(geometry.bubble.top);
        const center=(geometry.badges[0].left+geometry.badges[0].right)/2,bubbleCenter=(geometry.bubble.left+geometry.bubble.right)/2;
        expect(geometry.sent?center<bubbleCenter:center>bubbleCenter).toBe(true);
    }
    await jump(57);await expect(row(57).locator('.reaction-badge')).toHaveCount(0);
    await jump(50);
    const bubble=row(50).locator('.bubble'),before=await bubble.boundingBox();
    await row(50).getByRole('button',{name:/reacted/}).first().click();
    const dialog=page.getByRole('dialog',{name:'Reactions',exact:true});
    await expect(dialog).toBeVisible();
    for(const name of ['You','Demo Beryl01 Example','Demo Cobalt02 Example'])await expect(dialog.getByText(name,{exact:true})).toBeVisible();
    await mkdir('../lab/evidence/private/web',{recursive:true});
    await page.screenshot({path:`../lab/evidence/private/web/transcript-who-reacted-${info.project.name}.png`});
    await page.keyboard.press('Escape');await expect(dialog).not.toBeVisible();
    expect(await bubble.boundingBox()).toEqual(before);
    await mkdir('../lab/evidence/private/web',{recursive:true});
    await page.screenshot({path:`../lab/evidence/private/web/transcript-reactions-${info.project.name}.png`});
});
test('WEB-MSG-REACTIONS transcript keeps emoji and sticker reactions on their own message part',async({page,request})=>{
    await prepareSession(page);
    const ids:Record<string,string>=await control(request,'/demo-fixtures');
    const fixture:{message:string}=await control(request,'/reaction-fixture');
    await page.goto('/messages/'+encodeURIComponent(ids['message-chat'])+'/'+encodeURIComponent(fixture.message));
    const row=page.locator('[data-message="'+fixture.message+'"]');
    await expect(row.locator('[data-part="0"]').locator('..').getByRole('button',{name:/reacted/})).toHaveCount(1);
    await expect(row.locator('[data-part="1"]').locator('..').getByRole('button',{name:/reacted/})).toHaveCount(2);
    const sticker=row.getByRole('button',{name:/reacted Sticker/});
    await expect(sticker.locator('[data-sticker-placeholder]')).toBeVisible();
    await sticker.click();
    const dialog=page.getByRole('dialog',{name:'Reactions',exact:true});
    await expect(dialog).toContainText('Demo Beryl01 Example');
    await expect(dialog).toContainText('🪁');
    await expect(dialog).not.toContainText('❤️');
});
