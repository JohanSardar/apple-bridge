import { test as base, expect } from '@playwright/test';
import { bootstrap } from './session';
import { mkdir,writeFile } from 'node:fs/promises';
const test=base.extend<{avatar:{record:string;digest:string}}>({avatar:[async({context,request},use,info)=>{
    await bootstrap(context,request);const response=await request.post(process.env.AB_WEB_CONTROL!+'/contact-avatar',{headers:{'X-Bridge-Lab':process.env.AB_WEB_CONTROL_KEY!},timeout:180000});expect(response.ok()).toBe(true);const value=await response.json();
    expect(value.fixture_flush_budget_ms).toBe(180000);expect(value.fixture_flush_ms).toBeGreaterThanOrEqual(0);
    await mkdir('../lab/evidence/private/web/fixtures',{recursive:true});await writeFile('../lab/evidence/private/web/fixtures/avatar-'+info.project.name+'.json',JSON.stringify({fixture_flush_ms:value.fixture_flush_ms,fixture_flush_budget_ms:value.fixture_flush_budget_ms}));await use(value);
},{timeout:180000}]});
test('WEB-CONTACT-AVATAR-THUMB contact images use authorized thumb route',async({page,avatar})=>{
    const url=`/web/v1/blob/${avatar.digest}?record=${encodeURIComponent(avatar.record)}&variant=thumb`;
    const response=await page.request.get(url);
    expect(response.status()).toBe(200);
    expect(response.headers()['content-type']).toBe('image/png');
    expect(response.headers()['cache-control']).toBe('private, max-age=31536000, immutable');
    expect((await response.body()).subarray(1,4).toString()).toBe('PNG');
    expect((await page.request.get(url.replace(avatar.digest,'0'.repeat(64)))).status()).toBe(401);
    expect((await page.request.get(url.replace('variant=thumb','variant=invalid'))).status()).toBe(400);
});
