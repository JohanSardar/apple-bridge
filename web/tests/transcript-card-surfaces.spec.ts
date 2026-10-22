import {test,expect,type BrowserContext} from '@playwright/test';
import {mkdir} from 'node:fs/promises';
import {control} from './session';
let cookies:Parameters<BrowserContext['addCookies']>[0]=[],ids:Record<string,string>,pairs:Record<string,string>;
test.beforeAll(async({request})=>{cookies=[(await control(request,'/session')).cookie];ids=await control(request,'/demo-fixtures');pairs=await control(request,'/rich-parts-fixture');});
test('WEB-MSG-CARD-SURFACES sent and received cards retain a distinct surface and continuous tail in both appearances',async({page,context},info)=>{
    await context.addCookies(cookies);
    for(const direction of ['sent','received']){
        await page.goto('/messages/'+encodeURIComponent(ids['message-chat'])+'/'+encodeURIComponent(pairs[direction+'Text']));await expect(page.locator('.messages-transcript')).toHaveAttribute('aria-busy','false');await expect(page.locator('.transcript')).toHaveCSS('transform','none');
        const row=page.locator('[data-message="'+pairs[direction+'Card']+'"]'),card=row.locator('.message-card');await expect(card).toBeVisible();
        for(const colorScheme of ['light','dark'] as const){await page.emulateMedia({colorScheme});await expect(page.locator('html')).toHaveAttribute('data-theme',colorScheme);await expect(card).toHaveCSS('background-color',colorScheme==='dark'?'rgb(44, 44, 46)':'rgb(242, 242, 247)');const edge=await card.evaluate(element=>getComputedStyle(element,'::after').boxShadow);expect(edge).not.toBe('none');expect(await row.locator('.bubble-tail path').evaluate(element=>getComputedStyle(element).fill)).toBe(await card.evaluate(element=>getComputedStyle(element).backgroundColor));await mkdir('../lab/evidence/private/web',{recursive:true});await page.screenshot({path:`../lab/evidence/private/web/transcript-card-surface-${direction}-${colorScheme}-${info.project.name}.png`});}
    }
});
test('WEB-MSG-CARD-SURFACES decoded link and app artwork keep the same card surface without changing geometry',async({page,context})=>{
    await context.addCookies(cookies);
    for(const index of [100,101]){const id=ids['demo-message-00-'+String(index).padStart(6,'0')];await page.goto('/messages/'+encodeURIComponent(ids['message-chat'])+'/'+encodeURIComponent(id));await expect(page.locator('.messages-transcript')).toHaveAttribute('aria-busy','false');await expect(page.locator('.transcript')).toHaveCSS('transform','none');const card=page.locator('[data-message="'+id+'"] .message-card');await expect(card).toHaveAttribute('data-card-kind',index===100?'link':'app');const before=await card.boundingBox();for(const colorScheme of ['light','dark'] as const){await page.emulateMedia({colorScheme});await expect(card).toHaveCSS('background-color',colorScheme==='dark'?'rgb(44, 44, 46)':'rgb(242, 242, 247)');expect(await card.boundingBox()).toEqual(before);}}
});
test('WEB-MSG-CARD-SURFACES missing Genmoji bytes render an honest reserved tile rather than a bare glyph',async({page,context},info)=>{
    await context.addCookies(cookies);await page.goto('/messages/'+encodeURIComponent(ids['message-chat'])+'/'+encodeURIComponent(pairs.missingGenmoji));await expect(page.locator('.messages-transcript')).toHaveAttribute('aria-busy','false');await expect(page.locator('.transcript')).toHaveCSS('transform','none');
    const bubble=page.locator('[data-message="'+pairs.missingGenmoji+'"] .bubble'),tile=bubble.getByRole('img',{name:'Unavailable synthetic Genmoji'});await expect(tile).toBeVisible();await expect(tile).toHaveText('');await expect(tile.locator('svg')).toHaveCount(1);const rect=await tile.boundingBox(),reserved=await bubble.boundingBox();expect(rect).toEqual(reserved);expect(rect!.height).toBe(info.project.name.endsWith('-phone')?22:16);await expect(tile).not.toHaveCSS('background-color','rgba(0, 0, 0, 0)');await mkdir('../lab/evidence/private/web',{recursive:true});await page.screenshot({path:`../lab/evidence/private/web/transcript-missing-genmoji-${info.project.name}.png`});
});
