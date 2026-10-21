import { createHash, randomBytes } from 'node:crypto';
const csp = "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' blob: data:; media-src 'self' blob:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'";
function frame(data, opcode = 1) {
  const body = Buffer.from(typeof data === 'string' ? data : JSON.stringify(data));
  const head = Buffer.alloc(body.length < 126 ? 2 : 4);
  head[0] = 128 | opcode;
  head[1] = body.length < 126 ? body.length : 126;
  if (body.length >= 126) head.writeUInt16BE(body.length, 2);
  return Buffer.concat([head, body]);
}
export function mockWeb() {
  const sessions = new Map();
  const sockets = new Set();
  let sequence = 1;
  let snapshot = { chats: [] };
  const cursor = () => ({ epoch: 'fixture', subscription_revision: 1, sequence });
  const sessionFor = req => sessions.get(/ab_fixture=([^;]+)/.exec(req.headers.cookie || '')?.[1]);
  function attach(server) {
    server.middlewares.use(async (req, res, next) => {
      res.setHeader('Content-Security-Policy', csp);
      res.setHeader('Referrer-Policy', 'no-referrer');
      res.setHeader('X-Content-Type-Options', 'nosniff');
      const url = new URL(req.url, 'http://localhost');
      const path = url.pathname;
      if (path === '/__mock/session') {
        const id = randomBytes(12).toString('hex');
        const kind = url.searchParams.get('kind') || 'trusted';
        const session = { device_id: id, kind, cache: kind === 'public' ? 'memory' : 'persistent', expires_at: new Date(Date.now()+86400000).toISOString() };
        sessions.set(id, session);
        res.setHeader('Set-Cookie', `ab_fixture=${id}; HttpOnly; SameSite=Strict; Path=/`);
        res.writeHead(200, {'Content-Type':'application/json'}).end(JSON.stringify(session));
        return;
      }
      if (path === '/__mock/revoke') {
        const session = sessionFor(req);
        if (session) {
          for (const client of sockets) if (client.session.device_id === session.device_id) { client.socket.write(frame({t:'session_revoked'})); client.socket.end(); }
          sessions.delete(session.device_id);
        }
        res.end('{}'); return;
      }
      if (path === '/__mock/drop') { for (const client of sockets) client.socket.destroy(); res.end('{}'); return; }
      if (path === '/__mock/delta') {
        sequence++;
        const row = { id: 'fixture-' + sequence, title: 'Fixture update ' + sequence };
        snapshot.chats.push(row);
        for (const client of sockets) for (const id of client.subs) client.socket.write(frame({t:'delta',sub:id,changes:[{op:'upsert',row}],cursor:cursor()}));
        res.end(JSON.stringify(row)); return;
      }
      if (!path.startsWith('/web/v1/')) { next(); return; }
      let body = '';
      for await (const part of req) body += part;
      let input;
      try { input = body ? JSON.parse(body) : {}; } catch { res.writeHead(400).end(); return; }
      res.setHeader('Content-Type','application/json');
      if (req.method !== 'GET' && req.headers.origin !== `http://${req.headers.host}`) { res.writeHead(403).end('{}'); return; }
      const session = sessionFor(req);
      if (path === '/web/v1/login/options' || path === '/web/v1/enroll/options') {
        res.end(JSON.stringify({ publicKey:{ challenge: randomBytes(32).toString('base64url'), rpId:'localhost', userVerification:'required' } })); return;
      }
      if (!session) { res.writeHead(401).end('{}'); return; }
      if (path === '/web/v1/session') { res.end(JSON.stringify(session)); return; }
      if (path === '/web/v1/logout') { sessions.delete(session.device_id); res.end('{}'); return; }
      if (path === '/web/v1/devices') { res.end(JSON.stringify({ devices: [...sessions.values()].map(s=>({...s,id:s.device_id,name:'Fixture browser'})) })); return; }
      if (path === '/web/v1/passkeys') { res.end(JSON.stringify({ passkeys:[] })); return; }
      if (path.startsWith('/web/v1/devices/') && req.method === 'DELETE') {
        const id = decodeURIComponent(path.split('/').at(-1));
        sessions.delete(id);
        for (const client of sockets) if (client.session.device_id === id) { client.socket.write(frame({t:'session_revoked'})); client.socket.end(); }
        res.end('{}'); return;
      }
      void input;
      res.writeHead(404).end('{}');
    });
    server.httpServer.on('upgrade',(req,socket)=>{
      if (req.url !== '/web/v1/live') return;
      const session = sessionFor(req);
      if (!session || req.headers.origin !== `http://${req.headers.host}`) { socket.end('HTTP/1.1 403 Forbidden\r\n\r\n'); return; }
      const accept = createHash('sha1').update(req.headers['sec-websocket-key']+'258EAFA5-E914-47DA-95CA-C5AB0DC85B11').digest('base64');
      socket.write(`HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ${accept}\r\nSec-WebSocket-Protocol: web.1\r\n\r\n`);
      const client = {socket,session,subs:new Set()};
      sockets.add(client);
      socket.on('close',()=>sockets.delete(client));
      socket.on('error',()=>sockets.delete(client));
      let buffer = Buffer.alloc(0);
      socket.on('data',chunk=>{
        buffer = Buffer.concat([buffer,chunk]);
        while (buffer.length >= 2) {
          const opcode = buffer[0]&15;
          let length = buffer[1]&127, offset = 2;
          if (length === 127) { socket.destroy(); return; }
          if (length === 126) { if (buffer.length<4) return; length=buffer.readUInt16BE(2); offset=4; }
          const masked = Boolean(buffer[1]&128);
          if (buffer.length < offset+(masked?4:0)+length) return;
          const mask = buffer.subarray(offset,offset+4);
          if (masked) offset+=4;
          const bytes = Buffer.from(buffer.subarray(offset,offset+length));
          if (masked) for (let i=0;i<length;i++) bytes[i]^=mask[i%4];
          buffer=buffer.subarray(offset+length);
          if (opcode===8) { socket.end(); return; }
          if (opcode===9) { socket.write(frame(bytes.toString(),10)); continue; }
          let message;
          try { message=JSON.parse(bytes.toString()); } catch { socket.destroy(); return; }
          if (message.t==='hello') socket.write(frame({t:'welcome',session,head:cursor()}));
          if (message.t==='sub') { client.subs.add(message.id); socket.write(frame({t:'snap',sub:message.id,data:snapshot,cursor:cursor()})); }
          if (message.t==='unsub') client.subs.delete(message.id);
          if (message.t==='cmd') socket.write(frame({t:'ack',id:message.id,status:'admitted'}));
          if (message.t==='ping') socket.write(frame({t:'pong'}));
        }
      });
    });
  }
  return { name:'apple-bridge-dev-fixtures', configureServer:attach, configurePreviewServer:attach };
}
