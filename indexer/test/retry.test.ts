import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { retryWithBackoff, backoffDelayMs, DEFAULT_RETRY } from "../src/retry.js";

/**
 * Unit coverage for the retry-with-backoff helper the pollers wrap their RPC
 * fetch step in (issue #569). If any of this regresses, either a transient
 * blip kills the worker or a permanent failure busy-loops it — both worth a
 * red test.
 */
describe("retryWithBackoff", () => {
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

  it("resolves on the first attempt without logging a retry", async () => {
    const fn = vi.fn(async () => "ok");

    await expect(retryWithBackoff(fn, { attempts: 3, baseDelayMs: 1 })).resolves.toBe("ok");
    expect(fn).toHaveBeenCalledTimes(1);
    expect(logged).toHaveLength(0);
  });

  it("retries transient failures and resolves", async () => {
    let calls = 0;
    const fn = vi.fn(async (): Promise<string> => {
      calls += 1;
      if (calls === 1) throw new Error("ECONNRESET");
      if (calls === 2) throw new Error("ETIMEDOUT");
      return "recovered";
    });

    await expect(
      retryWithBackoff(fn, { attempts: 3, baseDelayMs: 1, maxDelayMs: 2, label: "getEvents" })
    ).resolves.toBe("recovered");
    expect(fn).toHaveBeenCalledTimes(3);
    expect(logged).toHaveLength(2);
  });

  it("emits one structured warn line per retry with attempt counters", async () => {
    const fn = vi.fn(async () => {
      throw new Error("rpc 503");
    });

    await expect(
      retryWithBackoff(fn, { attempts: 3, baseDelayMs: 1, label: "fetchEvents" })
    ).rejects.toThrow("rpc 503");

    expect(logged).toHaveLength(2); // attempts 1 and 2 retry; attempt 3 gives up
    const first = JSON.parse(logged[0]);
    expect(first.level).toBe("warn");
    expect(first.label).toBe("fetchEvents");
    expect(first.attempt).toBe(1);
    expect(first.maxAttempts).toBe(3);
    expect(first.delayMs).toBe(1);
    expect(first.error).toContain("rpc 503");

    const second = JSON.parse(logged[1]);
    expect(second.attempt).toBe(2);
    expect(second.delayMs).toBe(2);
  });

  it("rethrows the original error once attempts are exhausted", async () => {
    const boom = new Error("persistent failure");
    const fn = vi.fn(async () => {
      throw boom;
    });

    await expect(retryWithBackoff(fn, { attempts: 3, baseDelayMs: 1 })).rejects.toBe(boom);
    expect(fn).toHaveBeenCalledTimes(3);
  });

  it("honours attempts: 1 (no retry at all)", async () => {
    const fn = vi.fn(async () => {
      throw new Error("nope");
    });

    await expect(retryWithBackoff(fn, { attempts: 1, baseDelayMs: 1 })).rejects.toThrow("nope");
    expect(fn).toHaveBeenCalledTimes(1);
    expect(logged).toHaveLength(0);
  });

  it("stops retrying when the signal aborts mid-backoff", async () => {
    const controller = new AbortController();
    const fn = vi.fn(async () => {
      throw new Error("boom");
    });

    await expect(
      retryWithBackoff(fn, {
        attempts: 10,
        baseDelayMs: 10_000,
        signal: controller.signal,
        onRetry: () => controller.abort(),
      })
    ).rejects.toThrow("boom");
    expect(fn).toHaveBeenCalledTimes(1);
    expect(logged).toHaveLength(1); // the backoff sleep returned immediately
  });

  it("does not start a fetch at all if the signal is already aborted", async () => {
    const controller = new AbortController();
    controller.abort();
    const fn = vi.fn(async () => "never");

    await expect(
      retryWithBackoff(fn, { attempts: 3, baseDelayMs: 1, signal: controller.signal })
    ).rejects.toThrow(/aborted before first attempt/);
    expect(fn).not.toHaveBeenCalled();
  });
});

describe("backoffDelayMs", () => {
  it("doubles per attempt (base, 2x, 4x, …)", () => {
    expect(backoffDelayMs(1, 100, 10_000)).toBe(100);
    expect(backoffDelayMs(2, 100, 10_000)).toBe(200);
    expect(backoffDelayMs(3, 100, 10_000)).toBe(400);
    expect(backoffDelayMs(4, 100, 10_000)).toBe(800);
  });

  it("caps at maxDelayMs instead of growing unbounded", () => {
    expect(backoffDelayMs(10, 250, 5_000)).toBe(5_000);
    expect(backoffDelayMs(30, 250, 5_000)).toBe(5_000);
  });

  it("exposes sane defaults for the pollers", () => {
    expect(DEFAULT_RETRY.attempts).toBeGreaterThanOrEqual(1);
    expect(backoffDelayMs(1)).toBe(DEFAULT_RETRY.baseDelayMs);
    expect(backoffDelayMs(99)).toBe(DEFAULT_RETRY.maxDelayMs);
  });
});
