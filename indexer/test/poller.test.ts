import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import type { Pool, PoolClient } from "pg";
import { getCursor, saveCursor, ingestPage, pollOnce, startPollLoop } from "../src/poller.js";
import { setupGracefulShutdown } from "../src/worker.js";
import type { RawEvent } from "../src/handlers.js";

// ---------------------------------------------------------------------------
// Helpers: mocked PoolClient / Pool that records queries and simulates
// minimal Postgres behaviour needed for poller tests.
// ---------------------------------------------------------------------------

function mockClient(overrides: Partial<PoolClient> = {}): PoolClient & { queries: string[] } {
  const queries: string[] = [];
  const client = {
    query: vi.fn(async (sql: string, _params?: unknown[]) => {
      queries.push(sql);
      // Simulate cursor table
      if (sql.includes("SELECT last_ledger FROM cursor")) {
        return { rows: [{ last_ledger: 42 }], rowCount: 1 } as never;
      }
      if (sql.includes("SELECT pg_try_advisory_lock")) {
        return { rows: [{ locked: true }], rowCount: 1 } as never;
      }
      // BEGIN/COMMIT/ROLLBACK etc
      return { rows: [], rowCount: 0 } as never;
    }),
    release: vi.fn(),
    on: vi.fn(),
    queries,
    ...overrides,
  } as unknown as PoolClient & { queries: string[] };
  return client;
}

function mockPool(client: PoolClient): Pool {
  return {
    connect: vi.fn(async () => client),
  } as unknown as Pool;
}

const sampleEvent = (overrides: Partial<RawEvent> = {}): RawEvent => ({
  ledger: 100,
  txHash: "abc123",
  eventType: "stream_withdrawn",
  contractId: "CA3D...",
  topics: ["stream_withdrawn", "G..."],
  data: {
    stream_id: "1",
    recipient: "GRECIPIENT",
    amount: "1000",
    sender: "GSENDER",
    token: "GTOKEN",
    deposit: "100000",
    rate_per_second: "10",
  },
  ...overrides,
});

/** Poll `predicate` until true or fail — used for the loop-based tests. */
async function waitFor(predicate: () => boolean, timeoutMs = 2000): Promise<void> {
  const startedAt = Date.now();
  while (!predicate()) {
    if (Date.now() - startedAt > timeoutMs) {
      throw new Error(`waitFor: condition not met within ${timeoutMs}ms`);
    }
    await new Promise((resolve) => setTimeout(resolve, 5));
  }
}

describe("poller", () => {
  let client: ReturnType<typeof mockClient>;

  beforeEach(() => {
    client = mockClient();
    vi.clearAllMocks();
  });

  describe("getCursor", () => {
    it("returns last_ledger from cursor table", async () => {
      const cursor = await getCursor(client);
      expect(cursor).toBe(42);
      expect(client.query).toHaveBeenCalledWith(
        expect.stringContaining("SELECT last_ledger FROM cursor")
      );
    });

    it("returns 0 when cursor row missing", async () => {
      client.query = vi.fn(async () => ({ rows: [], rowCount: 0 } as never));
      const cursor = await getCursor(client);
      expect(cursor).toBe(0);
    });
  });

  describe("saveCursor", () => {
    it("upserts cursor with GREATEST guard (idempotent)", async () => {
      await saveCursor(client, 99);
      const sql = (client.query as ReturnType<typeof vi.fn>).mock.calls[0][0] as string;
      expect(sql).toContain("INSERT INTO cursor");
      expect(sql).toContain("ON CONFLICT (id) DO UPDATE");
      expect(sql).toContain("GREATEST");
    });

    it("requires a PoolClient (transactional) — signature enforces same-tx usage", async () => {
      // This is a compile-time guarantee; at runtime we just verify it uses
      // the passed client rather than acquiring a new one.
      await saveCursor(client, 100);
      expect(client.query).toHaveBeenCalledTimes(1);
      // No pool.connect inside saveCursor — it must be called within the
      // caller's BEGIN/COMMIT block. See pollOnce for the correct pattern.
    });
  });

  describe("ingestPage", () => {
    it("inserts raw_events and folds each event", async () => {
      const events = [sampleEvent({ ledger: 101 }), sampleEvent({ ledger: 102, txHash: "def456" })];
      await ingestPage(client, events);
      const calls = (client.query as ReturnType<typeof vi.fn>).mock.calls;
      // Each event triggers at least 2 queries: raw_events insert + handler upsert
      expect(calls.length).toBeGreaterThanOrEqual(4);
      const allSql = calls.map((c) => c[0] as string).join("\n");
      expect(allSql).toContain("INSERT INTO raw_events");
      expect(allSql).toContain("ON CONFLICT (ledger, tx_hash, event_type, contract_id) DO NOTHING");
    });

    it("is idempotent — duplicate page does not error", async () => {
      const events = [sampleEvent()];
      await ingestPage(client, events);
      // Re-ingest same page — ON CONFLICT DO NOTHING keeps it safe
      await expect(ingestPage(client, events)).resolves.not.toThrow();
    });
  });

  describe("pollOnce — transactional cursor atomicity (the gap fix)", () => {
    it("wraps ingestPage + saveCursor in a single transaction (BEGIN/COMMIT)", async () => {
      const events = [sampleEvent({ ledger: 50, txHash: "tx1" })];
      const fetchEvents = vi.fn(async () => events);
      const pool = mockPool(client);

      // Make getCursor return 42, so fromLedger = 42, nextLedger = 50
      client.query = vi.fn(async (sql: string) => {
        if (sql.includes("SELECT last_ledger")) return { rows: [{ last_ledger: 42 }], rowCount: 1 } as never;
        return { rows: [], rowCount: 0 } as never;
      }) as never;

      const result = await pollOnce(pool, fetchEvents, 100);
      expect(result.fetched).toBe(1);
      expect(result.nextLedger).toBe(50);

      const calls = (client.query as ReturnType<typeof vi.fn>).mock.calls.map((c) => c[0] as string);
      // Must be: BEGIN -> ingestPage queries -> saveCursor -> COMMIT
      // Previously poller did ingestPage(pool) + saveCursor(pool) as two
      // separate commits, leaving a crash window. Now both are inside one tx.
      const beginIdx = calls.findIndex((s) => s === "BEGIN");
      const commitIdx = calls.findIndex((s) => s === "COMMIT");
      const saveCursorIdx = calls.findIndex((s) => s.includes("INSERT INTO cursor"));
      expect(beginIdx).toBeGreaterThanOrEqual(0);
      expect(commitIdx).toBeGreaterThan(beginIdx);
      expect(saveCursorIdx).toBeGreaterThan(beginIdx);
      expect(saveCursorIdx).toBeLessThan(commitIdx);
    });

    it("rolls back on ingest failure (no partial cursor advance)", async () => {
      const events = [sampleEvent({ ledger: 51 })];
      const fetchEvents = vi.fn(async () => events);
      const pool = mockPool(client);

      client.query = vi.fn(async (sql: string) => {
        if (sql.includes("SELECT last_ledger")) return { rows: [{ last_ledger: 50 }], rowCount: 1 } as never;
        if (sql.includes("INSERT INTO raw_events")) throw new Error("inject fault");
        return { rows: [], rowCount: 0 } as never;
      }) as never;

      await expect(pollOnce(pool, fetchEvents)).rejects.toThrow("inject fault");
      const calls = (client.query as ReturnType<typeof vi.fn>).mock.calls.map((c) => c[0] as string);
      expect(calls).toContain("ROLLBACK");
      expect(calls).not.toContain("COMMIT");
    });

    it("does not advance cursor when no events fetched", async () => {
      const fetchEvents = vi.fn(async () => []);
      const pool = mockPool(client);
      client.query = vi.fn(async (sql: string) => {
        if (sql.includes("SELECT last_ledger")) return { rows: [{ last_ledger: 10 }], rowCount: 1 } as never;
        return { rows: [], rowCount: 0 } as never;
      }) as never;

      const result = await pollOnce(pool, fetchEvents);
      expect(result.fetched).toBe(0);
      expect(result.nextLedger).toBeNull();
      const calls = (client.query as ReturnType<typeof vi.fn>).mock.calls.map((c) => c[0] as string);
      expect(calls).not.toContain("BEGIN");
      expect(calls).not.toContain("COMMIT");
    });

    it("uses GREATEST in saveCursor so out-of-order delivery does not rewind", async () => {
      await saveCursor(client, 100);
      await saveCursor(client, 90);
      // Both calls generate the same idempotent SQL — the DB's GREATEST
      // ensures 90 does not overwrite 100.
      const calls = (client.query as ReturnType<typeof vi.fn>).mock.calls;
      expect(calls[0][0]).toContain("GREATEST");
      expect(calls[1][0]).toContain("GREATEST");
    });
  });

  describe("startPollLoop & graceful shutdown", () => {
    it("exits startPollLoop when AbortSignal is triggered", async () => {
      const fetchEvents = vi.fn(async () => []);
      const pool = mockPool(client);
      client.query = vi.fn(async (sql: string) => {
        if (sql.includes("SELECT last_ledger")) return { rows: [{ last_ledger: 10 }], rowCount: 1 } as never;
        return { rows: [], rowCount: 0 } as never;
      }) as never;

      const abortController = new AbortController();
      // Abort after 50ms
      setTimeout(() => abortController.abort(), 50);

      const loopPromise = startPollLoop(pool, fetchEvents, {
        intervalMs: 1000,
        signal: abortController.signal,
      });

      await expect(loopPromise).resolves.toBeUndefined();
      expect(abortController.signal.aborted).toBe(true);
    });

    it("setupGracefulShutdown awaits in-flight batch and releases advisory lock", async () => {
      const lockClient = mockClient();
      const pool = {
        connect: vi.fn(),
        end: vi.fn(async () => undefined),
      } as unknown as Pool;

      const exitSpy = vi.spyOn(process, "exit").mockImplementation((() => undefined) as never);
      const abortController = new AbortController();

      let batchFinished = false;
      const fakeBatchPromise = new Promise<void>((resolve) => {
        setTimeout(() => {
          batchFinished = true;
          resolve();
        }, 30);
      });

      const handleShutdown = setupGracefulShutdown(
        pool,
        lockClient,
        abortController,
        () => fakeBatchPromise
      );

      await handleShutdown("SIGTERM");

      expect(abortController.signal.aborted).toBe(true);
      expect(batchFinished).toBe(true);
      expect(lockClient.query).toHaveBeenCalledWith(
        expect.stringContaining("SELECT pg_advisory_unlock"),
        expect.any(Array)
      );
      expect(lockClient.release).toHaveBeenCalled();
      expect(pool.end).toHaveBeenCalled();
      expect(exitSpy).toHaveBeenCalledWith(0);

      exitSpy.mockRestore();
    });
  });

  // -----------------------------------------------------------------------
  // Issue #569 — explicit retry-with-backoff around the RPC fetch step, and
  // proof the loop never silently stops advancing after a transient failure.
  // -----------------------------------------------------------------------
  describe("retry & backoff around the fetch step (issue #569)", () => {
    beforeEach(() => {
      // retryWithBackoff logs a structured warn line per attempt; keep it out
      // of the test output.
      vi.spyOn(console, "error").mockImplementation(() => undefined);
    });

    afterEach(() => {
      vi.restoreAllMocks();
    });

    it("retries a transient fetch failure inside pollOnce and still advances the cursor", async () => {
      let attempts = 0;
      const fetchEvents = vi.fn(async (): Promise<RawEvent[]> => {
        attempts += 1;
        if (attempts === 1) throw new Error("socket hang up");
        return [sampleEvent({ ledger: 70, txHash: "tx70" })];
      });
      const pool = mockPool(client);
      client.query = vi.fn(async (sql: string) => {
        if (sql.includes("SELECT last_ledger")) return { rows: [{ last_ledger: 42 }], rowCount: 1 } as never;
        return { rows: [], rowCount: 0 } as never;
      }) as never;

      const result = await pollOnce(pool, fetchEvents, 100, {
        retry: { attempts: 3, baseDelayMs: 1, maxDelayMs: 2 },
      });

      // The fetch was retried (not abandoned after one blip) …
      expect(fetchEvents).toHaveBeenCalledTimes(2);
      // … and the poll still produced a committed cursor advance.
      expect(result).toEqual({ fetched: 1, nextLedger: 70 });
      const sqls = (client.query as ReturnType<typeof vi.fn>).mock.calls.map((c) => c[0] as string);
      expect(sqls).toContain("BEGIN");
      expect(sqls).toContain("COMMIT");
    });

    it("rethrows after exhausting fetch retries without touching the cursor", async () => {
      const fetchEvents = vi.fn(async () => {
        throw new Error("rpc down");
      });
      const pool = mockPool(client);
      client.query = vi.fn(async () => ({ rows: [], rowCount: 0 } as never)) as never;

      await expect(
        pollOnce(pool, fetchEvents, 100, { retry: { attempts: 3, baseDelayMs: 1, maxDelayMs: 2 } })
      ).rejects.toThrow("rpc down");

      expect(fetchEvents).toHaveBeenCalledTimes(3);
      const sqls = (client.query as ReturnType<typeof vi.fn>).mock.calls.map((c) => c[0] as string);
      // Nothing was ingested and the cursor was not moved — the retry did not
      // turn a hard failure into a silent partial advance.
      expect(sqls).not.toContain("BEGIN");
      expect(sqls).not.toContain("COMMIT");
    });

    it("startPollLoop keeps advancing after an exhausted fetch failure (no silent stall)", async () => {
      let fetchCount = 0;
      const fetchEvents = vi.fn(async (): Promise<RawEvent[]> => {
        fetchCount += 1;
        // First pollOnce: attempts 1+2 both fail (retries exhausted) → the
        // loop's own backoff kicks in; from attempt 3 the RPC recovers.
        if (fetchCount <= 2) throw new Error("ECONNRESET");
        if (fetchCount === 3) return [sampleEvent({ ledger: 90, txHash: "tx90" })];
        return [];
      });
      const pool = mockPool(client);
      let committed = false;
      client.query = vi.fn(async (sql: string) => {
        if (sql.includes("SELECT last_ledger")) return { rows: [{ last_ledger: 42 }], rowCount: 1 } as never;
        if (sql === "COMMIT") committed = true;
        return { rows: [], rowCount: 0 } as never;
      }) as never;

      const controller = new AbortController();
      const loop = startPollLoop(pool, fetchEvents, {
        intervalMs: 5,
        retry: { attempts: 2, baseDelayMs: 1, maxDelayMs: 2 },
        signal: controller.signal,
      });

      // The failure is survived: a later iteration ingests and commits.
      await waitFor(() => committed);
      controller.abort();
      await loop;

      expect(fetchCount).toBeGreaterThanOrEqual(3);
      expect(committed).toBe(true);
    });

    it("startPollLoop backs off (not spins) after a hard fetch failure", async () => {
      let fetchCount = 0;
      const fetchEvents = vi.fn(async (): Promise<RawEvent[]> => {
        fetchCount += 1;
        if (fetchCount === 1) throw new Error("db blip");
        return [];
      });
      const pool = mockPool(client);
      client.query = vi.fn(async (sql: string) => {
        if (sql.includes("SELECT last_ledger")) return { rows: [{ last_ledger: 42 }], rowCount: 1 } as never;
        return { rows: [], rowCount: 0 } as never;
      }) as never;

      const controller = new AbortController();
      const loop = startPollLoop(pool, fetchEvents, {
        intervalMs: 500, // loop-level backoff is min(intervalMs, 2000)
        retry: { attempts: 1, baseDelayMs: 1 },
        signal: controller.signal,
      });

      await waitFor(() => fetchCount >= 2);
      controller.abort();
      await loop;

      // After the failed iteration the loop slept before polling again
      // instead of hot-looping the failing cursor (a hot loop would have
      // burned through thousands of fetches in that window).
      expect(fetchCount).toBeLessThan(10);
    });
  });
});

