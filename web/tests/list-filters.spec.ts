import { expect, type Page } from '@playwright/test';
import { test } from './list-session';
import { control } from './session';
import { mkdir } from 'node:fs/promises';
const choices=[['All Messages','all','Maya Chen'],['Known Senders','known','Maya Chen'],['Unknown Senders','unknown','Unknown sender fixture'],['Unread','unread','Maya Chen'],['Recently Deleted','deleted','Deleted fixture'],['Spam','spam','Spam fixture']] as const;
async function choose(page:Page,label:string){await page.getByRole('button',{name:'Filters and Edit',exact:true}).click();await page.getByRole('menuitemradio',{name:label,exact:true}).click();await expect(page.locator('.conversation-list')).toHaveAttribute('data-filter',label);}
test('WEB-MSG-FILTERS six filters keep stable server views',async({page,request})=>{
    await control(request,'/list-filters');const frames:any[]=[];page.on('websocket',socket=>socket.on('framesent',frame=>{try{frames.push(JSON.parse(String(frame.payload)));}catch{}}));
    await page.goto('/messages');const list=page.getByRole('listbox',{name:'Conversations'});await expect(list).toBeVisible();await expect.poll(async()=>Number(await list.locator('[aria-setsize]').first().getAttribute('aria-setsize'))).toBeGreaterThanOrEqual(540);
    await expect.poll(()=>frames.filter(frame=>frame.t==='sub'&&frame.view==='chats'&&!frame.args?.after).map(frame=>frame.args.filter).sort()).toEqual(['all','deleted','spam']);
    const baseline=frames.filter(frame=>frame.t==='sub'&&frame.view==='chats').map(frame=>frame.id).sort();
    for(const [label,value,title] of choices){
        await choose(page,label);await expect(page.locator('.conversation-row').filter({hasText:title}).first()).toBeVisible();
        expect(frames.filter(frame=>frame.t==='sub'&&frame.view==='chats').map(frame=>frame.id).sort()).toEqual(baseline);
        if(value!=='deleted')await expect(page.locator('.conversation-row').filter({hasText:'Deleted fixture'})).toHaveCount(0);
        if(value==='unknown')await expect(page.locator('.conversation-row').filter({hasText:'Maya Chen'})).toHaveCount(0);
        if(value==='unread')expect(await page.locator('.conversation-row').evaluateAll(rows=>rows.every(row=>row.querySelector('.unread-dot')))).toBe(true);
    }
});
test('WEB-MSG-FILTERS every filter has its own empty state',async({page,request},info)=>{
    await control(request,'/list-filters');await page.goto('/messages');await expect(page.getByRole('listbox',{name:'Conversations'})).toBeVisible();await control(request,'/list-empty');
    for(const [label,empty] of [['All Messages','No Messages'],['Known Senders','No Messages from Known Senders'],['Unknown Senders','No Messages from Unknown Senders'],['Unread','No Unread Messages'],['Recently Deleted','No Recently Deleted Messages'],['Spam','No Spam']]){
        await choose(page,label);await expect(page.locator('.list-empty')).toHaveText(empty);await expect(page.locator('.conversation-row')).toHaveCount(0);if(label==='All Messages'){await mkdir('../lab/evidence/private/web',{recursive:true});await page.screenshot({path:`../lab/evidence/private/web/list-empty-${info.project.name}.png`});}
    }
});
test('WEB-MSG-FILTERS menu supports keyboard dismissal and outside clicks',async({page},info)=>{
    await page.goto('/messages');await expect(page.getByRole('listbox',{name:'Conversations'})).toBeVisible();const trigger=page.getByRole('button',{name:'Filters and Edit',exact:true});
    await trigger.focus();await trigger.press('ArrowDown');const menu=page.getByRole('menu',{name:'Message filters'});await expect(menu).toBeVisible();await mkdir('../lab/evidence/private/web',{recursive:true});await page.screenshot({path:`../lab/evidence/private/web/list-filters-${info.project.name}.png`});await expect(menu.getByRole('menuitemradio',{name:'All Messages',exact:true})).toBeFocused();
    await page.keyboard.press('End');await expect(menu.getByRole('menuitemradio',{name:'Spam',exact:true})).toBeFocused();await page.keyboard.press('Home');await page.keyboard.press('ArrowDown');await expect(menu.getByRole('menuitemradio',{name:'Known Senders',exact:true})).toBeFocused();await page.keyboard.press('Enter');await expect(menu).toHaveCount(0);await expect(trigger).toBeFocused();
    await trigger.click();await trigger.evaluate(element=>(element as HTMLElement).blur());await expect(trigger).not.toBeFocused();await page.keyboard.press('Escape');await expect(menu).toHaveCount(0);await expect(trigger).toBeFocused();await trigger.click();await page.getByRole('searchbox',{name:'Search',exact:true}).click();await expect(menu).toHaveCount(0);
});
