import { mkdtempSync, readFileSync, writeFileSync, rmSync } from 'node:fs';
import { randomBytes } from 'node:crypto';
import { tmpdir } from 'node:os';
import { resolve, join } from 'node:path';
import { execFileSync } from 'node:child_process';

const root = resolve(import.meta.dirname, '../..');
const provisioned = process.argv[2];
if (!provisioned) throw Error('provide the provisioned 1.7.17 accounts bundle');
const scratch = mkdtempSync(join(tmpdir(), 'ab-account-fixtures-'));
try {
  for (const [mode, name] of [['fixture', 'auth.mjs'], ['production', 'accounts.mjs']]) {
    execFileSync(process.execPath, [join(import.meta.dirname, 'reuse-accounts-bundle.mjs'), provisioned, join(scratch, name), mode]);
  }
  for (const name of ['accounts.test.ts', 'domain-migration.test.ts']) {
    const source = readFileSync(join(root, 'services/accounts/test', name), 'utf8')
      .replace("import { toNodeHandler } from 'better-auth/node';", "import { toNodeHandler } from './auth.mjs';")
      .replace("'../src/auth.ts'", "'./auth.mjs'")
      .replace("join(import.meta.dirname, '../../../rust/crates/ab-relay-control/assets/account.js')", JSON.stringify(join(root, 'rust/crates/ab-relay-control/assets/account.js')));
    writeFileSync(join(scratch, name), source);
  }
  const secret = join(scratch, 'fixture-secret');
  writeFileSync(secret, randomBytes(48).toString('base64url'), { mode: 0o600 });
  execFileSync(process.execPath, ['--test', '--test-concurrency=1', join(scratch, 'accounts.test.ts'), join(scratch, 'domain-migration.test.ts')], {
    stdio: 'inherit', env: { ...process.env, AB_TEST_BUNDLE: join(scratch, 'accounts.mjs'), AB_TEST_SECRET_FILE: secret },
  });
} finally {
  rmSync(scratch, { recursive: true, force: true });
}
