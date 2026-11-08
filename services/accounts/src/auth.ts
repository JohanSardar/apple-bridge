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
  registrationOrigin?: string;
  allowRegistration?: boolean;
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

class ImmediateTransactions extends DatabaseSync {
  override prepare(sql: string) {
    return super.prepare(sql === 'begin' ? 'begin immediate' : sql);
  }
}

export function open(config: Config) {
  const database = new ImmediateTransactions(config.databasePath);
  database.exec('PRAGMA busy_timeout=5000;');
  for (let attempt = 0; ; attempt += 1) {
    try {
      database.exec('PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;');
      break;
    } catch (error) {
      if (attempt >= 7 || !(error instanceof Error) || !/database is locked/.test(error.message)) throw error;
      Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 25 * (attempt + 1));
    }
  }
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
            if (config.allowRegistration !== true || config.registrationOrigin) {
              throw new APIError('FORBIDDEN', { message: config.registrationOrigin ? `Create your account with an invite at ${config.registrationOrigin}, then sign in here.` : 'New accounts are disabled on this sign-in host.' });
            }
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
      for (let attempt = 0; ; attempt += 1) {
        try {
          const { runMigrations } = await getMigrations(auth.options);
          await runMigrations();
          return;
        } catch (error) {
          if (attempt >= 7 || !(error instanceof Error) || !/table .* already exists|database is locked/.test(error.message)) throw error;
          await new Promise(resolve => setTimeout(resolve, 25 * (attempt + 1)));
        }
      }
    },
  };
}
