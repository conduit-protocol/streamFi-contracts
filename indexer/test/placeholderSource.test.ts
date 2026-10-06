import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { StubSorobanEventSource } from "../src/indexer/eventSource.js";
import { fetchEventsFromRPC, warnPlaceholderSourceOnce } from "../src/worker.js";

/**
 * Issue #568 — a placeholder event source must be loud, not silent.
 *
 * Before this, `npm run start:worker` against the stub started cleanly, logged
 * nothing wrong, and simply never indexed anything: an operator could not tell
 * "intentionally running the scaffold" from "the real RPC implementation is
 * broken and returning zero events".
 */
describe("placeholder event source warning (issue #568)", () => {
  let warnings: string[];

  beforeEach(() => {
    warnings = [];
    vi.spyOn(console, "warn").mockImplementation((...args: unknown[]) => {
      warnings.push(args.map(String).join(" "));
    });
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("logs a one-time warning when StubSorobanEventSource is constructed", () => {
    new StubSorobanEventSource();

    expect(warnings).toHaveLength(1);
    const line = JSON.parse(warnings[0]);
    expect(line.msg).toContain("placeholder SorobanEventSource");
    expect(line.msg).toContain("no events will be indexed");
    expect(line.level).toBe("warn");
  });

  it("does not repeat the warning on every getEvents call", async () => {
    const source = new StubSorobanEventSource();
    expect(warnings).toHaveLength(1);

    for (const startLedger of [1, 2, 3]) {
      const page = await source.getEvents({ startLedger, limit: 100 });
      expect(page.events).toEqual([]);
      expect(page.nextToken).toBeNull();
      expect(page.lastLedger).toBe(startLedger);
    }

    // Still exactly one warning — construction, not per poll.
    expect(warnings).toHaveLength(1);
  });

  it("warns exactly once per process when the worker's placeholder fetch runs", async () => {
    warnPlaceholderSourceOnce();
    warnPlaceholderSourceOnce();

    expect(await fetchEventsFromRPC(1, 100)).toEqual([]);
    expect(await fetchEventsFromRPC(101, 100)).toEqual([]);

    expect(warnings).toHaveLength(1);
    const line = JSON.parse(warnings[0]);
    expect(line.msg).toContain("placeholder event source");
    expect(line.msg).toContain("no events will be indexed");
    expect(line.source).toBe("fetchEventsFromRPC");
  });
});
