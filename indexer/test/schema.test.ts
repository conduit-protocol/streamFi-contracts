import { describe, it, expect } from "vitest";
import { readFileSync } from "node:fs";

/**
 * Static checks against `db/schema.sql` (issues #566, #567): the derived
 * tables the fold handlers write to must actually exist in the schema, with
 * the columns the acceptance criteria name. A handler that references a table
 * nobody ships would only fail at runtime against a live Postgres — which CI
 * doesn't have — so assert it here instead.
 */
const schema = readFileSync(new URL("../db/schema.sql", import.meta.url), "utf8");

function tableBody(name: string): string {
  // Tables end with a line containing only `);`, and column defaults such as
  // `now()` contain parens of their own — so anchor on the terminator rather
  // than on the first `)`.
  const match = schema.match(
    new RegExp(`CREATE TABLE IF NOT EXISTS ${name} \\(([\\s\\S]*?)\\n\\);`)
  );
  if (!match) throw new Error(`table ${name} missing from indexer/db/schema.sql`);
  return match[1];
}

function hasColumn(body: string, column: string): boolean {
  return new RegExp(`^\\s*${column}\\s`, "m").test(body);
}

describe("indexer/db/schema.sql", () => {
  it("has the streams table from issue #567 with every required column", () => {
    const body = tableBody("streams");
    for (const column of [
      "id",
      "sender",
      "recipient",
      "token",
      "rate_per_second",
      "start_time",
      "end_time",
      "created_at",
    ]) {
      expect(hasColumn(body, column), `streams.${column} missing`).toBe(true);
    }
    expect(body).toContain("PRIMARY KEY");
  });

  it("has the stream_events table from issue #566", () => {
    const body = tableBody("stream_events");
    for (const column of ["stream_id", "event_type", "ledger", "tx_hash", "payload", "created_at"]) {
      expect(hasColumn(body, column), `stream_events.${column} missing`).toBe(true);
    }
    // Page re-delivery idempotency: the fold's ON CONFLICT target.
    expect(body).toContain("PRIMARY KEY (ledger, tx_hash, event_type, stream_id)");
  });

  it("keeps the pre-existing derived tables intact", () => {
    expect(tableBody("loan_proposals")).toBeTruthy();
    expect(tableBody("treasury_proposals")).toBeTruthy();
    expect(tableBody("raw_events")).toBeTruthy();
    expect(tableBody("indexer_cursor")).toBeTruthy();
  });
});
