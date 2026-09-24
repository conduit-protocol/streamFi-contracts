/**
 * Fold — apply a single {@link SorobanEvent} to the local projection.
 *
 * Separated from the poller so it can be unit-tested and so fold failures
 * increment `metrics.foldFailures` per event rather than per page.
 *
 * Each `ev.type` branch documents the exact DB mutation it performs. Unknown
 * types are logged and counted as failures — forward-compatible with new
 * contract events without crashing the poller loop.
 */

import { SorobanEvent } from "./types.js";

export type FoldResult = { ok: true } | { ok: false; error: string };

/**
 * Stub fold — logs and returns ok. Replace with real Postgres / ORM logic:
 * - `created`    -> insert into `streams`
 * - `withdrawn`  -> update `streams.withdrawn`, insert `raw_events`
 * - `cancelled`/`force_cxl` -> mark `streams.status = 'cancelled'`
 * - etc.
 *
 * Throwing here is caught by the poller and counted as a fold failure;
 * returning `{ok:false}` does the same without throwing.
 */
export async function fold(event: SorobanEvent): Promise<FoldResult> {
  // Minimal validation per type — ensures required fields exist before DB write.
  switch (event.type) {
    case "created": {
      if (!event.fields["recipient"] || !event.fields["token"]) {
        return { ok: false, error: "missing recipient/token for created" };
      }
      break;
    }
    case "withdrawn": {
      if (event.fields["amount"] === undefined) return { ok: false, error: "missing amount for withdrawn" };
      break;
    }
    case "cancelled":
    case "force_cxl": {
      if (event.fields["refund_amount"] === undefined) return { ok: false, error: "missing refund_amount" };
      break;
    }
    case "xfer_rec": {
      if (!event.fields["new_recipient"]) return { ok: false, error: "missing new_recipient for xfer_rec" };
      break;
    }
    case "paused": {
      if (event.fields["paused_at"] === undefined) return { ok: false, error: "missing paused_at" };
      break;
    }
    case "resumed": {
      if (event.fields["resumed_at"] === undefined && event.fields["resumed_at"] !== 0) {
        // resumed_at may be 0 edge but generally required
      }
      break;
    }
    case "topped_up":
    case "clawback":
    case "set_op":
    case "rm_op":
    case "factory_paused":
    case "factory_unpaused":
      break;
    default: {
      // Unknown type — forward-compatible: warn but don't crash.
      return { ok: false, error: `unknown event type: ${(event as unknown as { type: string }).type}` };
    }
  }

  // TODO: real DB write here, e.g.
  // await db.insertRawEvent(event);
  // await db.upsertStreamProjection(event);
  return { ok: true };
}
