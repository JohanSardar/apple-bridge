import { expect } from '@playwright/test';
import { test } from './list-session';
import { control } from './session';
test.beforeEach(async({page})=>{await page.goto('/messages');await expect(page.getByRole('listbox',{name:'Conversations'})).toBeVisible();});
test('WEB-MSG-SEARCH-LIST inline title and all participant matches require no search request',async({page})=>{
    const requests:string[]=[];page.on('websocket',socket=>socket.on('framesent',frame=>requests.push(String(frame.payload))));const search=page.getByRole('searchbox',{name:'Search',exact:true}),rows=page.locator('.conversation-row');
    await search.fill('MAYA');await expect(rows).toHaveCount(3);await expect(rows.first()).toContainText('Maya Chen');await expect(rows.nth(1)).toContainText('Weekend Crew');await expect(rows.nth(2)).toContainText('Conversation 014');
    await search.fill('sam');await expect(rows).toHaveCount(2);await expect(rows.filter({hasText:'Weekend Crew'})).toHaveCount(1);
    await search.fill('no matching conversation');await expect(page.locator('.list-empty')).toHaveText('No Results');await expect(rows).toHaveCount(0);
    await page.getByRole('button',{name:'Clear search',exact:true}).click();await expect(search).toHaveValue('');await expect(search).toBeFocused();await expect(rows.first()).toContainText('Maya Chen');expect(requests.some(raw=>{try{return JSON.parse(raw).view==='search';}catch{return false;}})).toBe(false);
});
test('WEB-MSG-SEARCH-LIST pinned matches, search shortcut and IME-safe full-results hook',async({page,request},info)=>{
    await control(request,'/list-pins');const pins=page.getByRole('group',{name:'Pinned conversations'});await expect(pins.getByRole('button')).toHaveCount(3);const search=page.getByRole('searchbox',{name:'Search',exact:true});
    if(info.project.use.viewport?.width===1440){const modifier=await page.evaluate(()=>/Mac|iPhone|iPad/.test(navigator.platform)?'Meta':'Control');await page.keyboard.press(modifier+'+f');await expect(search).toBeFocused();await search.press('Escape');await expect(search).not.toBeFocused();await page.keyboard.press(modifier+'+Alt+f');await expect(search).toBeFocused();}
    await search.fill('sam');await expect(pins.getByRole('button')).toHaveCount(1);await expect(pins.getByRole('button',{name:'Weekend Crew',exact:true})).toBeVisible();
    const before=page.url();await search.dispatchEvent('keydown',{key:'Enter',isComposing:true});expect(page.url()).toBe(before);await search.fill('maya & weekend');await search.press('Enter');await expect(page).toHaveURL(/messages\/search\?q=maya%20%26%20weekend$/);
    await expect(page.getByRole('button',{name:'Details',exact:true})).toHaveCount(0);if(info.project.use.viewport?.width===1440){const modifier=await page.evaluate(()=>/Mac|iPhone|iPad/.test(navigator.platform)?'Meta':'Control');const full=page.url();await page.keyboard.press(modifier+'+i');expect(page.url()).toBe(full);await expect(page.getByRole('complementary',{name:'Details',exact:true})).toHaveCount(0);}
    await page.reload();await expect(page).toHaveURL(/messages\/search\?q=maya%20%26%20weekend$/);await expect(page.locator('.conversation-search input')).toHaveValue('maya & weekend');
});
