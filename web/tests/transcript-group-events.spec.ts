import {test,expect} from '@playwright/test';
import {mkdir} from 'node:fs/promises';
import {prepareSession,control} from './session';
test('WEB-MSG-GROUP-EVENTS photo removal is not a member removal',async({page,request},info)=>{
    await prepareSession(page);await page.clock.setFixedTime(new Date('2026-10-20T16:00:00Z'));const ids:Record<string,string>=await control(request,'/demo-fixtures'),native=(index:number)=>'demo-message-00-'+String(index).padStart(6,'0');
    await page.goto('/messages/'+encodeURIComponent(ids['message-chat'])+'/'+encodeURIComponent(ids[native(127)]));
    const removed=page.locator('[data-message="'+ids[native(127)]+'"]');await expect(removed.locator('.system-line')).toHaveText('Demo Beryl01 Example removed the group photo');await expect(removed.locator('.bubble')).toHaveCount(0);
    await mkdir('../lab/evidence/private/web',{recursive:true});await page.screenshot({path:`../lab/evidence/private/web/transcript-group-events-${info.project.name}.png`});
    for(const index of [126,128]){const row=page.locator('[data-message="'+ids[native(index)]+'"]');await expect(row.locator('.system-line')).toHaveText(/ updated the conversation$/u);await expect(row.locator('.bubble')).toHaveCount(0);}
    await page.goto('/messages/'+encodeURIComponent(ids['message-chat'])+'/'+encodeURIComponent(ids[native(26)]));
    for(const [index,pattern] of [[26,/named the conversation “Demo Lantern Club”$/u],[27,/added Demo /u],[28,/removed Demo /u],[29,/left the conversation$/u],[30,/changed the group photo$/u]] as const){const row=page.locator('[data-message="'+ids[native(index)]+'"]');await expect(row.locator('.system-line')).toHaveText(pattern);await expect(row.locator('.bubble')).toHaveCount(0);}
});
