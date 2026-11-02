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
