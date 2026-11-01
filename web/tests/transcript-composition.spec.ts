import {test,expect,type BrowserContext} from '@playwright/test';
import {mkdir} from 'node:fs/promises';
import {control} from './session';
test.use({deviceScaleFactor:2});
let cookies:Parameters<BrowserContext['addCookies']>[0]=[];
test.beforeAll(async({request})=>{cookies=[(await control(request,'/session')).cookie];});
test('WEB-MSG-COMPOSITION a same-sender image run uses exact owning cells and opens the clicked source',async({page,context,request},info)=>{
    await context.addCookies(cookies);const ids=await control(request,'/demo-fixtures'),messages=[42,43,44,45].map(index=>ids['demo-message-00-'+String(index).padStart(6,'0')]);
    await page.goto('/messages/'+encodeURIComponent(ids['message-chat'])+'/'+encodeURIComponent(messages[0]));
    await expect(page.locator('.messages-transcript')).toHaveAttribute('aria-busy','false');await expect(page.locator('.transcript')).toHaveCSS('transform','none');
    const row=page.locator('.message-row').filter({has:page.locator('.media-stack')}),stack=row.locator('.media-stack'),cells=stack.locator('.media-stack-cell');
    await expect(stack).toBeVisible();await expect(cells).toHaveCount(4);await expect(row).not.toHaveAttribute('data-message',/.+/);await expect(row).not.toHaveAttribute('data-media-origin',/.+/);await expect(row).toHaveAttribute('data-message-group',messages.join(' '));await expect(row.locator('.bubble[data-media-origin]')).toHaveCount(0);
    expect(await cells.evaluateAll(elements=>elements.map(element=>element.getAttribute('data-message')))).toEqual(messages);
    const origins=await cells.evaluateAll(elements=>elements.map(element=>element.getAttribute('data-media-origin')));expect(origins.every(Boolean)).toBe(true);expect(new Set(origins).size).toBe(4);expect(await cells.evaluateAll(elements=>elements.map(element=>element.closest('[data-part]')?.getAttribute('data-part')))).toEqual(['0','0','0','0']);
    const box=await stack.boundingBox(),viewport=await page.locator('.messages-transcript').evaluate(element=>element.clientWidth);expect(box!.width).toBeLessThanOrEqual(info.project.name.endsWith('-phone')?viewport*.7:480);expect(box!.height).toBeLessThanOrEqual(info.project.name.endsWith('-phone')?page.viewportSize()!.height*.6:360);
    if(info.project.name.endsWith('-phone')){await cells.nth(0).locator('button.media-open').focus();await page.keyboard.press('ArrowRight');await expect(stack).toHaveAttribute('data-stack-index','1');await expect(stack).toHaveAttribute('data-stack-motion','settled');}
    const source=cells.nth(1),button=source.locator('button.media-open');await expect(source).not.toHaveAttribute('inert','');await expect.poll(()=>source.locator('img').evaluate((image:HTMLImageElement)=>image.complete&&image.naturalWidth>0)).toBe(true);await button.focus();await page.evaluate(()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve))));const before=await source.boundingBox();await button.evaluate(element=>element.blur());
    await mkdir('../lab/evidence/private/web',{recursive:true});await page.screenshot({path:`../lab/evidence/private/web/transcript-stack-${info.project.name}.png`});await button.click();const viewer=page.getByRole('dialog',{name:'Media viewer'});await expect(viewer).toHaveAttribute('data-media-message',messages[1]);await expect(source).toHaveCSS('opacity','0');expect(await source.boundingBox()).toEqual(before);await expect(cells.nth(0)).toHaveCSS('visibility','visible');
    await page.screenshot({path:`../lab/evidence/private/web/transcript-stack-viewer-${info.project.name}.png`});await page.keyboard.press('Escape');await expect(viewer).toBeHidden();await expect(source).toHaveCSS('opacity','1');expect(await source.boundingBox()).toEqual(before);await expect(button).toBeFocused();
});
test('WEB-MSG-COMPOSITION a non-leading image deep link preserves its actual selection and usable source',async({page,context,request})=>{
    await context.addCookies(cookies);const ids=await control(request,'/demo-fixtures'),message=ids['demo-message-00-000044'];await page.goto('/messages/'+encodeURIComponent(ids['message-chat'])+'/'+encodeURIComponent(message));
    await expect(page.locator('.messages-transcript')).toHaveAttribute('aria-busy','false');const target=page.locator('[data-message="'+message+'"]');await expect(target).toHaveCount(1);await expect(target).toBeInViewport();await expect(target).not.toHaveAttribute('inert','');expect(await target.evaluate(element=>element.closest('.virtual-row')?.getAttribute('aria-selected'))).toBe('true');await expect(target.locator('button.media-open')).toBeVisible();
});
