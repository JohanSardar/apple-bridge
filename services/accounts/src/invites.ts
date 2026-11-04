import { createHash, randomBytes, randomUUID } from 'node:crypto';
import type { DatabaseSync } from 'node:sqlite';

export type InviteRow = {
  id: string;
  note: string;
  uses_left: number;
  expires_at: number;
  created_at: number;
};

const hash = (token: string) => createHash('sha256').update(token).digest('hex');

export class Invites {
  readonly #db: DatabaseSync;

  constructor(db: DatabaseSync) {
    this.#db = db;
    db.exec(`CREATE TABLE IF NOT EXISTS invite(
      id TEXT PRIMARY KEY,
      token_hash TEXT UNIQUE NOT NULL,
      note TEXT NOT NULL,
      uses_left INTEGER NOT NULL,
      expires_at INTEGER NOT NULL,
      created_at INTEGER NOT NULL
    )`);
  }

  create(input: { uses: number; days: number; note?: string }, now = Date.now()) {
    if (!Number.isInteger(input.uses) || input.uses < 1) throw new Error('uses must be a positive whole number');
    if (!(input.days > 0)) throw new Error('days must be positive');
    const token = randomBytes(32).toString('base64url');
    const id = randomUUID();
    this.#db
      .prepare('INSERT INTO invite VALUES(?,?,?,?,?,?)')
      .run(id, hash(token), input.note ?? '', input.uses, now + input.days * 86_400_000, now);
    return { id, token };
  }

  valid(token: string, now = Date.now()) {
    return (
      this.#db
        .prepare('SELECT 1 FROM invite WHERE token_hash=? AND expires_at>? AND uses_left>0')
        .get(hash(token), now) !== undefined
    );
  }

  consume(token: string, now = Date.now()) {
    const changed = this.#db
      .prepare('UPDATE invite SET uses_left=uses_left-1 WHERE token_hash=? AND expires_at>? AND uses_left>0')
      .run(hash(token), now);
    return changed.changes === 1;
  }

  list(): InviteRow[] {
    return this.#db
      .prepare('SELECT id,note,uses_left,expires_at,created_at FROM invite ORDER BY created_at')
      .all() as InviteRow[];
  }

  revoke(id: string) {
    return this.#db.prepare('UPDATE invite SET uses_left=0 WHERE id=?').run(id).changes === 1;
  }
}
