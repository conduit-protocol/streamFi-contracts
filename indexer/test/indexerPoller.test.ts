import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { Poller } from "../src/indexer/poller.js";
import type { Cursor, GetEventsParams, Page, SorobanEventSource } from "../src/indexer/types.js";

/**
 * Coverage for the class-based poller (`src/indexer/poller.ts`) — the
 * "read → fetch → fold → checkpoint" loop described in issue #569.
 *
 * The property under test: a single transient `getEvents` failure must never
 * leave the poller running-but-stuck. Either it retries and advances, or it
 * logs an error and the next tick picks the same cursor up again.
 */

const TINY_RETRY = { attempts: 3, baseDelayMs: 1, maxDelayMs: 2 } as const;

/** Poll `predicate` until true or fail — used for the interval-driven tests. */
async function waitFor(predicate: () => boolean, timeoutMs = 2000): Promise<void> {
  const startedAt = Date.now();
  while (!predicate()) {
    if (Date.now() - startedAt > timeoutMs) {
      throw new Error(`waitFor: condition not met within ${timeoutMs}ms`);
    }
    await new Promise((resolve) => setTimeout(resolve, 5));
  }
}

function page(overrides: Partial<Page> = {}): Page {
  return { events: [], nextToken: null, lastLedger: 500, ...overrides };
}

describe("Poller (src/indexer/poller.ts)", () => {
  let logged: string[];

  beforeEach(() => {
    logged = [];
    vi.spyOn(console, "error").mockImplementation((...args: unknown[]) => {
      logged.push(args.map(String).join(" "));
    });
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("retries a transient getEvents failure and checkpoints the cursor anyway", async () => {
    let calls = 0;
    const source: SorobanEventSource = {
      getEvents: vi.fn(async (): Promise<Page> => {
        calls += 1;
        if (calls === 1) throw new Error("503 service unavailable");
        return page();
      }),
    };
    const saveCursor = vi.fn(async (_c: Cursor) => undefined);

    const poller = new Poller({
      source,
      startLedger: 100,
      intervalMs: 60_000,
      retry: TINY_RETRY,
      saveCursor,
    });
    await poller.start();
    poller.stop();

    // Retried within the same tick — the blip did not cost a poll cycle …
    expect(calls).toBe(2);
    // … and the cursor still advanced, so the loop keeps making progress.
    expect(saveCursor).toHaveBeenCalledTimes(1);
    expect(saveCursor).toHaveBeenCalledWith({ lastLedger: 500, nextToken: null });
    // The retry was visible in the logs, not silent.
    expect(logged.some((line) => line.includes("retrying with backoff"))).toBe(true);
  });

  it("logs an error and leaves the cursor untouched when every attempt fails", async () => {
    const source: SorobanEventSource = {
      getEvents: vi.fn(async (): Promise<Page> => {
        throw new Error("rpc down");
      }),
    };
    const saveCursor = vi.fn(async (_c: Cursor) => undefined);

    const poller = new Poller({
      source,
      startLedger: 100,
      intervalMs: 60_000,
      retry: { attempts: 2, baseDelayMs: 1, maxDelayMs: 2 },
      saveCursor,
    });
    await poller.start();
    poller.stop();

    expect(source.getEvents).toHaveBeenCalledTimes(2);
    expect(saveCursor).not.toHaveBeenCalled();
    const failure = logged.find((line) => line.includes("getEvents failed after retries"));
    expect(failure).toBeDefined();
    expect(JSON.parse(failure as string).level).toBe("error");
  });

  it("does not silently stall: a later tick recovers and advances the cursor", async () => {
    let calls = 0;
    const source: SorobanEventSource = {
      getEvents: vi.fn(async (): Promise<Page> => {
        calls += 1;
        if (calls === 1) throw new Error("first tick explodes");
        return page();
      }),
    };
    const saveCursor = vi.fn(async (_c: Cursor) => undefined);

    const poller = new Poller({
      source,
      startLedger: 100,
      intervalMs: 5,
      // No in-tick retries: the first tick fails outright, so recovery can
      // only come from the loop itself ticking again.
      retry: { attempts: 1, baseDelayMs: 1 },
      saveCursor,
    });
    await poller.start();
    expect(saveCursor).not.toHaveBeenCalled();

    // The interval tick keeps firing after the failed tick — the poller must
    // pick the same cursor up and advance it.
    await waitFor(() => saveCursor.mock.calls.length > 0);
    poller.stop();

    expect(saveCursor).toHaveBeenCalledWith({ lastLedger: 500, nextToken: null });
    expect(calls).toBeGreaterThanOrEqual(2);
  });

  it("starts against the stub source without throwing and reports health", async () => {
    // The stub returns empty pages forever; the poller must still complete a
    // tick and mark it successful (that's what keeps /healthz green).
    const { StubSorobanEventSource } = await import("../src/indexer/eventSource.js");
    vi.spyOn(console, "warn").mockImplementation(() => undefined);

    const saveCursor = vi.fn(async (_c: Cursor) => undefined);
    const poller = new Poller({
      source: new StubSorobanEventSource(),
      startLedger: 1,
      intervalMs: 60_000,
      retry: TINY_RETRY,
      saveCursor,
    });
    await poller.start();
    poller.stop();

    // Stub echoes its scanned high-water mark: params.startLedger (cursor
    // starts at startLedger - 1, so the first fetch scans from startLedger).
    expect(saveCursor).toHaveBeenCalledWith({ lastLedger: 1, nextToken: null });
    expect(logged.some((line) => line.includes('"phase":"tick"'))).toBe(true);
  });

  it("threads the requested params into getEvents", async () => {
    const seen: GetEventsParams[] = [];
    const source: SorobanEventSource = {
      getEvents: vi.fn(async (params: GetEventsParams): Promise<Page> => {
        seen.push(params);
        return page();
      }),
    };
    const poller = new Poller({
      source,
      startLedger: 42,
      intervalMs: 60_000,
      limit: 25,
      retry: TINY_RETRY,
    });
    await poller.start();
    poller.stop();

    expect(seen).toHaveLength(1);
    expect(seen[0].startLedger).toBe(42);
    expect(seen[0].limit).toBe(25);
    expect(seen[0].cursor).toBeNull();
  });
});
