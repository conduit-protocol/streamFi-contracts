/**
 * Retry with exponential backoff — shared by every step that talks to an
 * upstream that can fail transiently (Soroban RPC `getEvents`, Postgres).
 *
 * Why this exists (issue #569): the poller's read → fetch → ingest → save
 * loop used to have *no* visible retry strategy around the RPC fetch. A
 * single connection blip either bubbled out as an unhandled rejection (the
 * worker dies) or was swallowed (the worker keeps running but never advances
 * its cursor again) — both undiagnosable from logs.
 *
 * Contract:
 *  - `fn` is retried up to `attempts` times *in total* (first call included).
 *  - Delay before retry N is `baseDelayMs * 2^(N-1)`, capped at `maxDelayMs`
 *    (deterministic — no jitter, so backoff behaviour is reproducible in logs
 *    and tests).
 *  - Every retry emits one structured JSON warn line on stderr with the
 *    attempt number, so "stalled" and "retrying" look different in logs.
 *  - When `signal` aborts (worker shutdown) the loop stops early and the
 *    last error is rethrown; the caller decides whether that is fatal.
 *  - The final failure rethrows the *original* error — no wrapping, so
 *    callers/tests can still match on `err.message`.
 */

/** Backoff policy shared by the pollers (see `PollerOptions`, `startPollLoop`). */
export interface RetryPolicy {
  /** Total attempts including the first. Default 3, minimum 1. */
  attempts?: number;
  /** Delay before the first retry, in ms. Default 250. Doubles per attempt. */
  baseDelayMs?: number;
  /** Upper bound for a single delay, in ms. Default 5000. */
  maxDelayMs?: number;
}

export interface RetryOptions extends RetryPolicy {
  /** Aborts the retry loop early (in-flight shutdown). */
  signal?: AbortSignal;
  /** Short label for the structured retry log line, e.g. `getEvents`. */
  label?: string;
  /** Observability hook — called before each backoff sleep. */
  onRetry?: (info: {
    attempt: number;
    maxAttempts: number;
    delayMs: number;
    error: unknown;
  }) => void;
}

export const DEFAULT_RETRY: Required<RetryPolicy> = {
  attempts: 3,
  baseDelayMs: 250,
  maxDelayMs: 5000,
};

/**
 * Delay before `attempt`'s retry (1-based): `base * 2^(attempt - 1)`, capped.
 * attempt=1 → base, attempt=2 → 2*base, attempt=3 → 4*base, …
 */
export function backoffDelayMs(
  attempt: number,
  baseDelayMs: number = DEFAULT_RETRY.baseDelayMs,
  maxDelayMs: number = DEFAULT_RETRY.maxDelayMs
): number {
  const delay = baseDelayMs * 2 ** Math.max(0, attempt - 1);
  return Math.min(delay, maxDelayMs);
}

/** Sleep that resolves early (without throwing) when `signal` aborts. */
export function sleep(ms: number, signal?: AbortSignal): Promise<void> {
  return new Promise((resolve) => {
    if (signal?.aborted) {
      resolve();
      return;
    }
    const onAbort = () => {
      clearTimeout(timer);
      resolve();
    };
    const timer = setTimeout(() => {
      signal?.removeEventListener("abort", onAbort);
      resolve();
    }, ms);
    signal?.addEventListener("abort", onAbort, { once: true });
  });
}

/**
 * Run `fn`, retrying transient failures with exponential backoff.
 * Throws the last error once `attempts` is exhausted (or the signal aborts).
 */
export async function retryWithBackoff<T>(
  fn: () => Promise<T>,
  opts: RetryOptions = {}
): Promise<T> {
  const maxAttempts = Math.max(1, opts.attempts ?? DEFAULT_RETRY.attempts);
  const baseDelayMs = opts.baseDelayMs ?? DEFAULT_RETRY.baseDelayMs;
  const maxDelayMs = opts.maxDelayMs ?? DEFAULT_RETRY.maxDelayMs;
  const label = opts.label ?? "retry";

  let lastError: unknown;
  for (let attempt = 1; attempt <= maxAttempts; attempt++) {
    if (opts.signal?.aborted) break;
    try {
      return await fn();
    } catch (error) {
      lastError = error;
      if (attempt === maxAttempts) break;

      const delayMs = backoffDelayMs(attempt, baseDelayMs, maxDelayMs);
      opts.onRetry?.({ attempt, maxAttempts, delayMs, error });
      // Structured warn line: a retrying worker must never look "healthy but
      // idle" — this is the line that distinguishes the two in logs.
      console.error(
        JSON.stringify({
          level: "warn",
          msg: "transient failure, retrying with backoff",
          label,
          attempt,
          maxAttempts,
          delayMs,
          error: String(error),
        })
      );
      await sleep(delayMs, opts.signal);
    }
  }
  // Aborted before the first attempt ever ran — surface that as a real error
  // rather than `throw undefined`.
  if (lastError === undefined) {
    throw new Error(`${label}: retry aborted before first attempt`);
  }
  throw lastError;
}
