import {expect,type APIRequestContext,type BrowserContext,type Locator} from '@playwright/test';
import {control} from './session';
let session:Promise<{cookie:Parameters<BrowserContext['addCookies']>[0][number]}>|undefined;
export async function mediaSession(context:BrowserContext,request:APIRequestContext){const value=await(session??=control(request,'/session?kind=public'));await context.addCookies([value.cookie]);}

export async function mediaTranscriptReady(media:Locator){await expect.poll(()=>media.evaluate(element=>Number(element.closest('.virtual-row')?.getAttribute('aria-setsize')))).toBeGreaterThan(100);await expect.poll(()=>media.evaluate(element=>element.closest('.virtual-row')?.querySelectorAll('[data-motion]').length??-1)).toBe(0);}
