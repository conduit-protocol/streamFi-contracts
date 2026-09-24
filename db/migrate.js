#!/usr/bin/env node
/**
 * Minimal hand-rolled numbered-file migration runner.
 *
 * - Applies `db/migrations/*.sql` in lexicographic order.
 * - Tracks applied versions in `schema_migrations` (idempotent).
 * - No external deps beyond `pg` (same as indexer). For `node-pg-migrate`,
 *   replace this file with that tool — the `db/migrations/` format is
 *   compatible (plain SQL files with numeric prefix).
 *
 * Usage:
 *   DATABASE_URL=postgres://user:pass@localhost:5432/streamfi node db/migrate.js up
 *   DATABASE_URL=... node db/migrate.js status
 *   DATABASE_URL=... node db/migrate.js down --last  # reverts last (if down file exists)
 *
 * The `down` path expects a companion file `XXX_name.down.sql` if you need rollback.
 * The initial migration is forward-only and down is a no-op (advisory).
 */

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import pg from "pg";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const MIGRATIONS_DIR = path.join(__dirname, "migrations");

async function getClient() {
  const url = process.env.DATABASE_URL;
  if (!url) {
    console.error("DATABASE_URL is required (e.g. postgres://user:pass@localhost:5432/streamfi)");
    process.exit(1);
  }
  const client = new pg.Client({ connectionString: url });
  await client.connect();
  return client;
}

async function ensureMigrationsTable(client) {
  await client.query(`
    CREATE TABLE IF NOT EXISTS schema_migrations (
      version TEXT PRIMARY KEY,
      applied_at TIMESTAMPTZ NOT NULL DEFAULT now()
    )
  `);
}

function listMigrations() {
  const files = fs.readdirSync(MIGRATIONS_DIR).filter((f) => f.endsWith(".sql") && !f.endsWith(".down.sql")).sort();
  return files.map((f) => ({ version: f.replace(/\.sql$/, ""), file: path.join(MIGRATIONS_DIR, f) }));
}

async function status() {
  const client = await getClient();
  try {
    await ensureMigrationsTable(client);
    const { rows } = await client.query("SELECT version FROM schema_migrations ORDER BY version");
    const applied = new Set(rows.map((r) => r.version));
    const all = listMigrations();
    for (const m of all) {
      console.log(`${applied.has(m.version) ? "applied  " : "pending  "} ${m.version}`);
    }
    if (all.length === 0) console.log("(no migrations found)");
  } finally {
    await client.end();
  }
}

async function up() {
  const client = await getClient();
  try {
    await ensureMigrationsTable(client);
    const { rows } = await client.query("SELECT version FROM schema_migrations ORDER BY version");
    const applied = new Set(rows.map((r) => r.version));
    const all = listMigrations();
    let ran = 0;
    for (const m of all) {
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
        console.error(`failed ${m.version}: ${e.message}`);
        throw e;
      }
    }
    if (ran === 0) console.log("up: nothing to apply — all migrations already applied.");
    else console.log(`up: applied ${ran} migration(s).`);
  } finally {
    await client.end();
  }
}

async function down() {
  // Minimal rollback: removes last applied version row and attempts to run
  // X.down.sql if present. Without a down file we just un-record the version
  // (advisory — does NOT actually undo DDL). For real down, add a companion file.
  const client = await getClient();
  try {
    await ensureMigrationsTable(client);
    const { rows } = await client.query("SELECT version FROM schema_migrations ORDER BY version DESC LIMIT 1");
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
      console.log(`down: no ${version}.down.sql — only removing history row (no DDL rollback). Add a .down.sql file for real rollback.`);
    }
    await client.query("DELETE FROM schema_migrations WHERE version = $1", [version]);
    await client.query("COMMIT");
    console.log(`down: reverted ${version}`);
  } finally {
    await client.end();
  }
}

const cmd = process.argv[2] ?? "up";
if (cmd === "status") status().catch((e) => { console.error(e); process.exit(1); });
else if (cmd === "up") up().catch((e) => { console.error(e); process.exit(1); });
else if (cmd === "down") down().catch((e) => { console.error(e); process.exit(1); });
else {
  console.error(`unknown command: ${cmd} (expected up|down|status)`);
  process.exit(1);
}
