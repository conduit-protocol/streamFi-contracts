#!/usr/bin/env tsx
/**
 * Indexer migration runner — thin wrapper over `db/migrate.js`.
 *
 * This file is the canonical "minimal migration tool" for the indexer.
 * It is a hand-rolled numbered-file runner (no `node-pg-migrate` required,
 * but the `db/migrations/*.sql` layout is compatible if you switch).
 *
 * - Migrations live in `db/migrations/*.sql` (repo root) and, for
 *   self-contained runs, are also mirrored under `indexer/db/migrations/` via
 *   the copy step in `npm run migrate`.
 * - History is tracked in `schema_migrations` (see `db/schema.sql`).
 *
 * Usage:
 *   DATABASE_URL=postgres://... npm run --prefix indexer migrate
 *   DATABASE_URL=... npm run --prefix indexer migrate:down
 *   DATABASE_URL=... npx tsx indexer/src/db/migrate.ts status
 */

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import pg from "pg";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

// Repo root is two levels up from indexer/src/db
const REPO_ROOT = path.resolve(__dirname, "../../../");
const MIGRATIONS_DIR = path.join(REPO_ROOT, "db/migrations");

function requireDatabaseUrl(): string {
  const url = process.env.DATABASE_URL;
  if (!url) {
    console.error("DATABASE_URL is required (e.g. postgres://user:pass@localhost:5432/streamfi)");
    process.exit(1);
  }
  return url;
}

async function ensureMigrationsTable(client: pg.Client): Promise<void> {
  await client.query(`
    CREATE TABLE IF NOT EXISTS schema_migrations (
      version TEXT PRIMARY KEY,
      applied_at TIMESTAMPTZ NOT NULL DEFAULT now()
    )
  `);
}

function listMigrations(): { version: string; file: string }[] {
  if (!fs.existsSync(MIGRATIONS_DIR)) return [];
  const files = fs.readdirSync(MIGRATIONS_DIR).filter((f) => f.endsWith(".sql") && !f.endsWith(".down.sql")).sort();
  return files.map((f) => ({ version: f.replace(/\.sql$/, ""), file: path.join(MIGRATIONS_DIR, f) }));
}

export async function status(): Promise<void> {
  const client = new pg.Client({ connectionString: requireDatabaseUrl() });
  await client.connect();
  try {
    await ensureMigrationsTable(client);
    const { rows } = await client.query<{ version: string }>("SELECT version FROM schema_migrations ORDER BY version");
    const applied = new Set(rows.map((r) => r.version));
    for (const m of listMigrations()) {
      console.log(`${applied.has(m.version) ? "applied  " : "pending  "} ${m.version}`);
    }
    if (listMigrations().length === 0) console.log("(no migrations found in db/migrations/)");
  } finally {
    await client.end();
  }
}

export async function up(): Promise<void> {
  const client = new pg.Client({ connectionString: requireDatabaseUrl() });
  await client.connect();
  try {
    await ensureMigrationsTable(client);
    const { rows } = await client.query<{ version: string }>("SELECT version FROM schema_migrations ORDER BY version");
    const applied = new Set(rows.map((r) => r.version));
    let ran = 0;
    for (const m of listMigrations()) {
      if (applied.has(m.version)) continue;
      const sql = fs.readFileSync(m.file, "utf8");
      console.log(`applying ${m.version} ...`);
      await client.query("BEGIN");
      try {
        await client.query(sql);
        await client.query("INSERT INTO schema_migrations(version) VALUES ($1)", [m.version]);
        await client.query("COMMIT");
        console.log(`applied ${m.version}`);
        ran++;
      } catch (e) {
        await client.query("ROLLBACK");
        console.error(`failed ${m.version}: ${(e as Error).message}`);
        throw e;
      }
    }
    if (ran === 0) console.log("up: nothing to apply — all migrations already applied.");
    else console.log(`up: applied ${ran} migration(s).`);
  } finally {
    await client.end();
  }
}

export async function down(): Promise<void> {
  const client = new pg.Client({ connectionString: requireDatabaseUrl() });
  await client.connect();
  try {
    await ensureMigrationsTable(client);
    const { rows } = await client.query<{ version: string }>("SELECT version FROM schema_migrations ORDER BY version DESC LIMIT 1");
    if (rows.length === 0) {
      console.log("down: no applied migrations.");
      return;
    }
    const version = rows[0].version;
    const downFile = path.join(MIGRATIONS_DIR, `${version}.down.sql`);
    await client.query("BEGIN");
    if (fs.existsSync(downFile)) {
      const sql = fs.readFileSync(downFile, "utf8");
      console.log(`reverting ${version} via ${downFile} ...`);
      await client.query(sql);
    } else {
      console.log(`down: no ${version}.down.sql — only removing history row (no DDL rollback).`);
    }
    await client.query("DELETE FROM schema_migrations WHERE version = $1", [version]);
    await client.query("COMMIT");
    console.log(`down: reverted ${version}`);
  } finally {
    await client.end();
  }
}

// CLI
if (import.meta.url === `file://${process.argv[1]}` || process.argv[1]?.endsWith("migrate.ts")) {
  const cmd = process.argv[2] ?? "up";
  if (cmd === "status") status().catch((e) => { console.error(e); process.exit(1); });
  else if (cmd === "up") up().catch((e) => { console.error(e); process.exit(1); });
  else if (cmd === "down") down().catch((e) => { console.error(e); process.exit(1); });
  else {
    console.error(`unknown command: ${cmd} (expected up|down|status)`);
    process.exit(1);
  }
}
