import {test,expect,type BrowserContext} from '@playwright/test';
import {control} from './session';
let cookies:Parameters<BrowserContext['addCookies']>[0]=[],ids:Record<string,string>;
test.beforeAll(async({request})=>{cookies=[(await control(request,'/session')).cookie];ids=await control(request,'/demo-fixtures');});
test('WEB-MSG-REPLY-HANDOFF a parent reply target closes the reader without stealing Composer focus',async({page,context})=>{
    await context.addCookies(cookies);const root=ids['demo-message-00-000006'];await page.goto('/__fixtures/reply-handoff?chat='+encodeURIComponent(ids['message-chat'])+'&anchor='+encodeURIComponent(root));const main=page.locator('.transcript-body>.messages-transcript');await expect(main).toHaveAttribute('aria-busy','false');const trigger=main.locator('[data-message="'+root+'"] button.reply-count');await trigger.click();const reader=page.getByRole('dialog',{name:'Replies',exact:true});await expect(reader).toBeVisible();await expect(main).toHaveAttribute('inert','');await expect(reader.getByRole('heading',{name:'Replies',exact:true})).toBeFocused();
    await page.getByRole('button',{name:'Set parent reply target',includeHidden:true}).dispatchEvent('click');await expect(reader).not.toBeVisible();await expect(trigger).not.toBeFocused();await expect(main).not.toHaveAttribute('inert');const composer=page.getByRole('textbox',{name:'Parent composer'});expect(await composer.evaluate(element=>Boolean(element.closest('[inert]')))).toBe(false);await composer.click();await expect(composer).toBeFocused();
});
