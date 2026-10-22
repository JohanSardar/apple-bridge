import { test, expect } from '@playwright/test';
import { prepareListSession } from './list-session';
import { mkdir } from 'node:fs/promises';
test('WEB-MSG-LIST demo reference screenshots',async({page,context,request},info)=>{
    await prepareListSession(context,request);await page.goto('/messages');await expect(page.getByRole('listbox',{name:'Conversations'})).toBeVisible();
    await expect.poll(()=>page.locator('.avatar img').count()).toBeGreaterThan(0);await page.locator('.avatar img').first().evaluate(async(image:HTMLImageElement)=>{await image.decode();});
    await mkdir('../lab/evidence/private/web',{recursive:true});await page.screenshot({path:`../lab/evidence/private/web/list-${info.project.name}.png`});
});
