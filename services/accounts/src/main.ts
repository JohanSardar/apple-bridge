import { randomBytes } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { createServer } from 'node:http';
import { toNodeHandler } from 'better-auth/node';
import { type Config, open } from './auth.ts';

function config(): Config {
  const need = (name: string) => {
    const value = process.env[name];
    if (!value) throw new Error(`${name} is required`);
    return value;
  };
  const secret = readFileSync(need('AB_ACCOUNTS_SECRET_FILE'), 'utf8').trim();
  if (secret.length < 32) throw new Error('the secret file must hold at least 32 characters');
  const { AB_GOOGLE_CLIENT_ID: clientId, AB_GOOGLE_CLIENT_SECRET: clientSecret } = process.env;
  return {
    publicUrl: need('AB_PUBLIC_URL'),
    databasePath: need('AB_ACCOUNTS_DB'),
    secret,
    maxAccounts: Number(process.env.AB_MAX_ACCOUNTS ?? 25),
    ...(clientId && clientSecret ? { google: { clientId, clientSecret } } : {}),
  };
}

async function serve() {
  const listen = process.env.AB_ACCOUNTS_LISTEN ?? '127.0.0.1:8790';
  const [host, port] = listen.split(':');
  if (host !== '127.0.0.1') throw new Error('the accounts service only listens on loopback');
  const accounts = open(config());
  await accounts.migrate();
  const handle = toNodeHandler(accounts.auth);
  const server = createServer((request, response) => {
    if (request.url === '/health') {
      accounts.database.prepare('SELECT 1').get();
      response.writeHead(200, { 'content-type': 'text/plain' }).end('ok\n');
      return;
    }
    if (!request.url?.startsWith('/api/auth/')) {
      response.writeHead(404).end();
      return;
    }
    void handle(request, response);
  });
  server.listen(Number(port), host, () => console.log(`accounts listening on ${listen}`));
  const stop = () => server.close(() => process.exit(0));
  process.on('SIGTERM', stop);
  process.on('SIGINT', stop);
}

function flag(args: string[], name: string, fallback: string) {
  const at = args.indexOf(`--${name}`);
  return at >= 0 ? (args[at + 1] ?? fallback) : fallback;
}

async function invite(args: string[]) {
  const settings = config();
  const accounts = open(settings);
  await accounts.migrate();
  switch (args[0]) {
    case 'create': {
      const { token } = accounts.invites.create({
        uses: Number(flag(args, 'uses', '1')),
        days: Number(flag(args, 'days', '7')),
        note: flag(args, 'note', ''),
      });
      console.log(`${new URL(settings.publicUrl).origin}/invite#${token}`);
      return;
    }
    case 'list':
      for (const row of accounts.invites.list()) {
        const expires = new Date(row.expires_at).toISOString();
        console.log(`${row.id}  uses left ${row.uses_left}  expires ${expires}  ${row.note}`);
      }
      return;
    case 'revoke':
      if (!args[1] || !accounts.invites.revoke(args[1])) throw new Error('no such invite');
      return;
    default:
      throw new Error('usage: invite create [--uses N] [--days N] [--note TEXT] | invite list | invite revoke ID');
  }
}

async function user(args: string[]) {
  const accounts = open(config());
  await accounts.migrate();
  const context = await accounts.auth.$context;
  switch (args[0]) {
    case 'list':
      for (const row of accounts.database.prepare('SELECT id,email,createdAt FROM user').all()) {
        console.log(`${row.id}  ${row.email}  ${row.createdAt}`);
      }
      return;
    case 'reset-password': {
      const found = args[1] ? await context.internalAdapter.findUserByEmail(args[1]) : null;
      if (!found) throw new Error('no such user');
      const password = randomBytes(18).toString('base64url');
      await context.internalAdapter.updatePassword(found.user.id, await context.password.hash(password));
      await context.internalAdapter.deleteSessions(found.user.id);
      console.log(password);
      return;
    }
    default:
      throw new Error('usage: user list | user reset-password EMAIL');
  }
}

const [command, ...rest] = process.argv.slice(2);
const commands: Record<string, () => Promise<void>> = {
  serve,
  invite: () => invite(rest),
  user: () => user(rest),
};
const run = commands[command ?? ''];
if (!run) {
  console.error('usage: accounts serve | invite ... | user ...');
  process.exit(2);
}
run().catch((error: unknown) => {
  console.error(error instanceof Error ? error.message : error);
  process.exit(1);
});
