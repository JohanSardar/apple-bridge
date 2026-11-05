import { passkey } from '@better-auth/passkey';
import { betterAuth, type BetterAuthPlugin } from 'better-auth';
import { APIError, createAuthEndpoint } from 'better-auth/api';
import { getMigrations } from 'better-auth/db/migration';
import { DatabaseSync } from 'node:sqlite';
import { Invites } from './invites.ts';

export type Config = {
  publicUrl: string;
  databasePath: string;
  secret: string;
  maxAccounts: number;
  google?: { clientId: string; clientSecret: string };
};

export const INVITE_COOKIE = '__Host-ab_invite';
export const SESSION_COOKIE = '__Host-ab_account';
const SESSION_DAYS = 7;

function inviteCheck(invites: Invites, secret: string): BetterAuthPlugin {
  return {
    id: 'ab-invite',
    endpoints: {
      checkInvite: createAuthEndpoint('/invite/check', { method: 'POST' }, async (ctx) => {
        const token = typeof ctx.body?.token === 'string' ? ctx.body.token : '';
        if (!invites.valid(token)) throw new APIError('FORBIDDEN', { message: 'This invite is no longer valid.' });
        await ctx.setSignedCookie(INVITE_COOKIE, token, secret, {
          httpOnly: true,
          secure: true,
          sameSite: 'lax',
          path: '/',
          maxAge: 3600,
        });
        return ctx.json({ valid: true });
      }),
    },
  };
}

export function open(config: Config) {
  const database = new DatabaseSync(config.databasePath);
  database.exec('PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000; PRAGMA foreign_keys=ON;');
  const invites = new Invites(database);
  const url = new URL(config.publicUrl);
  const auth = betterAuth({
    appName: 'Apple Bridge',
    baseURL: url.origin,
    basePath: '/api/auth',
    secret: config.secret,
    database,
    trustedOrigins: [url.origin],
    emailAndPassword: { enabled: true, minPasswordLength: 12, autoSignIn: true },
    ...(config.google ? { socialProviders: { google: config.google } } : {}),
    account: { accountLinking: { enabled: false } },
    user: { deleteUser: { enabled: false } },
    session: { expiresIn: SESSION_DAYS * 86_400, updateAge: 86_400, freshAge: 3600 },
    rateLimit: {
      enabled: true,
      window: 60,
      max: 60,
      customRules: {
        '/sign-in/email': { window: 60, max: 8 },
        '/sign-up/email': { window: 60, max: 8 },
        '/invite/check': { window: 60, max: 12 },
      },
    },
    advanced: {
      useSecureCookies: false,
      defaultCookieAttributes: { secure: true, httpOnly: true, sameSite: 'lax', path: '/' },
      ipAddress: { ipAddressHeaders: ['x-forwarded-for'] },
      cookies: {
        session_token: {
          name: SESSION_COOKIE,
          attributes: { path: '/', secure: true, httpOnly: true, sameSite: 'lax' },
        },
      },
    },
    databaseHooks: {
      user: {
        create: {
          before: async (user, context) => {
            const count = database.prepare('SELECT count(*) AS n FROM user').get() as { n: number };
            if (count.n >= config.maxAccounts) {
              throw new APIError('FORBIDDEN', { message: 'This relay is full.' });
            }
            const token = await context?.getSignedCookie(INVITE_COOKIE, config.secret);
            if (!token || !invites.consume(token)) {
              throw new APIError('FORBIDDEN', { message: 'You need an invite to create an account here.' });
            }
            return { data: user };
          },
        },
      },
    },
    plugins: [
      passkey({ rpID: url.hostname, rpName: 'Apple Bridge', origin: url.origin }),
      inviteCheck(invites, config.secret),
    ],
  });
  return {
    auth,
    invites,
    database,
    async migrate() {
      const { runMigrations } = await getMigrations(auth.options);
      await runMigrations();
    },
  };
}
