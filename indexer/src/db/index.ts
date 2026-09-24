/**
 * Minimal Postgres helpers for the indexer.
 *
 * Keeps cursor persistence separate from migration runner so the poller
 * can load/save `Cursor` without pulling the whole migration machinery.
 */

import pg from "pg";
import { Cursor } from "../indexer/types.js";

let pool: pg.Pool | null = null;

export function getPool(): pg.Pool {
  if (pool) return pool;
  const conn = process.env.DATABASE_URL;
  if (!conn) throw new Error("DATABASE_URL is required for indexer DB operations");
  pool = new pg.Pool({ connectionString: conn });
  return pool;
}

export async function loadCursor(): Promise<Cursor | null> {
  const p = getPool();
  const { rows } = await p.query<{ last_ledger: number; next_token: string | null }>(
    "SELECT last_ledger, next_token FROM indexer_cursor WHERE id = 1"
  );
  if (rows.length === 0) return null;
  return { lastLedger: rows[0].last_ledger, nextToken: rows[0].next_token };
}

export async function saveCursor(cursor: Cursor): Promise<void> {
  const p = getPool();
  await p.query(
    `INSERT INTO indexer_cursor (id, last_ledger, next_token, updated_at)
     VALUES (1, $1, $2, now())
     ON CONFLICT (id) DO UPDATE SET last_ledger = $1, next_token = $2, updated_at = now()`,
    [cursor.lastLedger, cursor.nextToken]
  );
}
