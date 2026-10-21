import type {APIRequestContext,BrowserContext} from '@playwright/test';
import {control} from './session';
let session:Promise<{cookie:Parameters<BrowserContext['addCookies']>[0][number]}>|undefined;
export async function mediaSession(context:BrowserContext,request:APIRequestContext){const value=await(session??=control(request,'/session?kind=public'));await context.addCookies([value.cookie]);}
