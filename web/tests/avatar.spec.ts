import { test, expect } from '@playwright/test';
import { bootstrap, control } from './session';
test('WEB-CONTACT-AVATAR-THUMB contact images use authorized thumb route',async({page,context,request})=>{
    await bootstrap(context,request);
    const avatar=await control(request,'/contact-avatar');
    const url=`/web/v1/blob/${avatar.digest}?record=${encodeURIComponent(avatar.record)}&variant=thumb`;
    const response=await page.request.get(url);
    expect(response.status()).toBe(200);
    expect(response.headers()['content-type']).toBe('image/png');
    expect(response.headers()['cache-control']).toBe('private, max-age=31536000, immutable');
    expect((await response.body()).subarray(1,4).toString()).toBe('PNG');
    expect((await page.request.get(url.replace(avatar.digest,'0'.repeat(64)))).status()).toBe(401);
    expect((await page.request.get(url.replace('variant=thumb','variant=invalid'))).status()).toBe(400);
});
