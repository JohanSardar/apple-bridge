const {chromium} = require(process.env.AB_PROOF_PLAYWRIGHT || 'playwright');
const fs = require('fs');
const cp = require('child_process');
const crypto = require('crypto');
(async()=>{
 const home=fs.mkdtempSync(require('path').join(require('os').tmpdir(),'ab-consent-'));
 const bridge=process.env.AB_PROOF_BRIDGE;const lab=process.env.AB_PROOF_LAB;
 if(!bridge || !lab)throw Error('Set AB_PROOF_BRIDGE and AB_PROOF_LAB to newly built binaries');
 const freePort=()=>new Promise((resolve,reject)=>{const server=require('net').createServer();server.on('error',reject);server.listen(0,'127.0.0.1',()=>{const port=server.address().port;server.close(()=>resolve(port));});});
 const port=await freePort();const callback='http://localhost:'+await freePort()+'/callback';
 const base='http://localhost:'+port;
 const env={...process.env,APPLE_BRIDGE_SECRET_STORE:'file'};
 const gateway=cp.spawn(bridge,['start','--home',home,'--api-addr','127.0.0.1:'+port,'--peer-addr','127.0.0.1:0'],{env,stdio:['ignore',fs.openSync(home+'/gateway.log','a'),fs.openSync(home+'/gateway.log','a')]});
 let browser,simulator;
 try {
  for(let i=0;i<100;i++){if(gateway.exitCode!==null)throw Error('Isolated gateway exited');try{if((await fetch(base)).ok)break;}catch{}await new Promise(r=>setTimeout(r,100));}
  simulator=cp.spawn(lab,['simapple','--socket',home+'/run/peer.sock','--secret-file',home+'/apple-secret','--control',home+'/control.sock'],{env,stdio:['ignore',fs.openSync(home+'/sim.log','a'),fs.openSync(home+'/sim.log','a')]});
  for(let i=0;i<200 && !fs.existsSync(home+'/control.sock');i++){if(simulator.exitCode!==null)throw Error('Synthetic peer exited');await new Promise(r=>setTimeout(r,100));}
  if(!fs.existsSync(home+'/control.sock'))throw Error('Synthetic peer not ready');
  const http=require('http');
  await new Promise((resolve,reject)=>{const request=http.request({socketPath:home+'/control.sock',path:'/messages',method:'POST',headers:{'Content-Type':'application/json'}},response=>{response.resume();response.on('end',resolve)});request.on('error',reject);request.end(JSON.stringify({action:'seed',count:0}));});
  const enroll=cp.execFileSync(bridge,['web','enroll','--home',home,'--host',base],{env,encoding:'utf8'}).split('\n')[0];
  browser=await chromium.launch({headless:true});
  const context=await browser.newContext();const page=await context.newPage();page.on('console',m=>{if(m.type()==='error')console.log('BROWSER',m.text())});
  const cdp=await context.newCDPSession(page);await cdp.send('WebAuthn.enable');await cdp.send('WebAuthn.addVirtualAuthenticator',{options:{protocol:'ctap2',transport:'internal',hasResidentKey:true,hasUserVerification:true,isUserVerified:true,automaticPresenceSimulation:true}});
  await page.goto(enroll);await page.locator('#signin').click();await page.getByText('Signed in. You may continue.').waitFor();
  console.log('PASS first passkey enrollment');
  const client=await (await fetch(base+'/oauth/register',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({client_name:'Bridge Test proof',redirect_uris:[callback],grant_types:['authorization_code','refresh_token'],token_endpoint_auth_method:'none'})})).json();
  if(!client.client_id)throw Error('registration');console.log('PASS dynamic registration');
  const verifier=crypto.randomBytes(32).toString('base64url');const q=new URLSearchParams({client_id:client.client_id,response_type:'code',redirect_uri:callback,scope:'messages:write',resource:base+'/mcp',state:'proof',code_challenge:crypto.createHash('sha256').update(verifier).digest('base64url'),code_challenge_method:'S256'});
  await context.clearCookies();await page.goto(base+'/oauth/authorize?'+q);await page.locator('#use-passkey').click();await page.locator('#signin').click();await page.getByRole('button',{name:'Allow',exact:true}).waitFor();await page.getByRole('radio',{name:/Read, and ask me/}).check();
  fs.mkdirSync(process.env.HOME+'/ab-live/proof/web-removal',{recursive:true});await page.screenshot({path:process.env.HOME+'/ab-live/proof/web-removal/consent.png'});
  let redirect;await page.route(callback+'**',async route=>{redirect=route.request().url();await route.fulfill({body:'Connector authorized'});});await page.getByRole('button',{name:'Allow',exact:true}).click();try { await page.waitForURL(callback+'**',{timeout:5000}); } catch(e) { throw e; }
  const code=new URL(redirect).searchParams.get('code');if(!code)throw Error('authorization');
  const token=await (await fetch(base+'/oauth/token',{method:'POST',body:new URLSearchParams({grant_type:'authorization_code',client_id:client.client_id,code,redirect_uri:callback,code_verifier:verifier,resource:base+'/mcp'})})).json();if(!token.access_token)throw Error('exchange');console.log('PASS passkey consent approve scope and token exchange');
  for(const [id,method,params] of [[1,'initialize',{protocolVersion:'2025-03-26',capabilities:{},clientInfo:{name:'proof',version:'1'}}],[2,'tools/list',{}]]){const r=await fetch(base+'/mcp',{method:'POST',headers:{Authorization:'Bearer '+token.access_token,'Content-Type':'application/json',Accept:'application/json, text/event-stream'},body:JSON.stringify({jsonrpc:'2.0',id,method,params})});const text=await r.text();if(r.status!==200||text.includes('"error"'))throw Error(method+': '+r.status+' '+text);console.log('PASS MCP '+method);}
  const op=async(name,input)=>{const r=await fetch(base+'/v1/op/'+name,{method:'POST',headers:{Authorization:'Bearer '+token.access_token,'Content-Type':'application/json'},body:JSON.stringify(input)});return {status:r.status,data:await r.json()};};
  let chats;
  for(let i=0;i<50;i++){chats=(await op('records.query',{collection:'messages.chat'})).data.records;if(chats?.length)break;await new Promise(r=>setTimeout(r,100));}
  if(!chats?.length)throw Error('synthetic chat');
  const id='cmd_'+Array.from(crypto.randomBytes(26),v=>'0123456789ABCDEFGHJKMNPQRSTVWXYZ'[v%32]).join('');
  const command=await op('messages.send',{command_id:id,args:{chat_id:chats[0].id,text:'[ab-test web-removal] synthetic proof only'},wait_ms:1});
  if(command.data.state!=='awaiting_approval')throw Error('awaiting approval '+JSON.stringify(command));
  console.log('PASS synthetic command awaiting_approval');
  await page.goto(base+'/approvals');await page.screenshot({path:process.env.HOME+'/ab-live/proof/web-removal/approvals.png'});console.log('PASS authenticated approvals page');
  await page.locator('form[action="/approvals/'+id+'"]').getByRole('button',{name:'Approve',exact:true}).click();
  await page.getByRole('heading',{name:'Command status'}).waitFor();
  let final;
  for(let i=0;i<100;i++){final=(await op('commands.status',{command_id:id})).data;if(final.state==='observed')break;await new Promise(r=>setTimeout(r,100));}
  if(final.state!=='observed')throw Error('final command '+final.state);
  await page.goto(base+'/approvals/'+id);
  await page.screenshot({path:process.env.HOME+'/ab-live/proof/web-removal/final-command.png'});
  console.log('PASS approved synthetic command observed (SimApple effect; production gateway)');
  for(const path of ['/web/v1/live','/web/v1/session','/web/v1/upload','/web/v1/handles','/web/v1/lookup','/web/v1/devices','/sw.js','/manifest.webmanifest'])if((await fetch(base+path)).status!==404)throw Error('removed '+path);
  console.log('PASS removed routes 404');
 }finally{await browser?.close();simulator?.kill('SIGTERM');gateway.kill('SIGTERM');}
})().catch(e=>{console.error(e);process.exitCode=1});
