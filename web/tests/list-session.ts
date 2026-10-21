import { test as base, expect } from '@playwright/test';
import { bootstrap } from './session';
import { mkdir, writeFile } from 'node:fs/promises';
export const test=base.extend<{listSetup:void}>({listSetup:[async({context,request},use,info)=>{
    await bootstrap(context,request);
    const response=await request.post(process.env.AB_WEB_CONTROL!+'/list-fixture',{headers:{'X-Bridge-Lab':process.env.AB_WEB_CONTROL_KEY!},timeout:180000});
    expect(response.ok()).toBe(true);
    const value=await response.json();
    await mkdir('../lab/evidence/private/web/reconnect',{recursive:true});
    await writeFile('../lab/evidence/private/web/reconnect/setup-'+info.project.name+'-'+info.testId.replace(/[^a-z0-9]/gi,'_')+'.json',JSON.stringify({project:info.project.name,test:info.title,fixture_setup_ms:value.fixture_setup_ms}));
    await use(undefined);
},{auto:true,timeout:180000}]});
