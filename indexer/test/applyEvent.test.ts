import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import type { PoolClient } from "pg";
import { applyEvent } from "../src/indexer/handlers.js";
import type { ChainEvent } from "../src/indexer/types.js";

function mockClient(): PoolClient {
  return {
    query: vi.fn(async () => ({ rows: [], rowCount: 1 })),
  } as unknown as PoolClient;
}

function ev(partial: Partial<ChainEvent> & Pick<ChainEvent, "type">): ChainEvent {
  return { fields: {}, ...partial };
}

/**
 * Regression coverage for issue #578: applyEvent's `default` branch used to
 * `return` without any log line, so an event type the fold didn't know about
 * (anything other than loan_vote / treasury_vote / treasury_reveal) vanished
 * without a trace. The default must log — not throw — and must not write to
 * the DB.
 */
describe("applyEvent", () => {
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

  function lastLogLine(): Record<string, unknown> {
    expect(logged.length).toBeGreaterThan(0);
    return JSON.parse(logged[logged.length - 1]);
  }

  it("folds the three DAO-voting event types without logging", async () => {
    const client = mockClient();

    await applyEvent(client, ev({ type: "loan_vote", fields: { proposal_id: 1, support: true } }));
    await applyEvent(client, ev({ type: "treasury_vote", fields: { proposal_id: 2, support: false } }));
    await applyEvent(client, ev({ type: "treasury_reveal", fields: { proposal_id: 3 } }));

    expect(client.query).toHaveBeenCalledTimes(3);
    expect(logged).toHaveLength(0);
  });

  it("logs a warn-level structured line for an unrecognized event type instead of dropping silently", async () => {
    const client = mockClient();

    await applyEvent(
      client,
      ev({ type: "dripstream_created", ledger: 123, contractId: "CABC", txHash: "deadbeef" }),
    );

    // No throw, no DB write — only the log line.
    expect(client.query).not.toHaveBeenCalled();
    expect(logged).toHaveLength(1);

    const line = lastLogLine();
    expect(line.level).toBe("warn");
    expect(line.msg).toContain("unrecognized event type");
    expect(line.type).toBe("dripstream_created");
    expect(line.ledger).toBe(123);
    expect(line.contractId).toBe("CABC");
    expect(line.txHash).toBe("deadbeef");
  });

  it("resolves without throwing even when the event carries no context fields", async () => {
    const client = mockClient();

    await expect(applyEvent(client, ev({ type: "totally_new_event" }))).resolves.toBeUndefined();

    expect(logged).toHaveLength(1);
    const line = lastLogLine();
    expect(line.type).toBe("totally_new_event");
    expect(line.ledger).toBeUndefined();
  });

  // ---------------------------------------------------------------------
  // Issues #566 / #567 — folds for the product's own events: stream
  // creation into `streams`, lifecycle events into `stream_events`.
  // ---------------------------------------------------------------------
  describe("stream-creation fold (issue #567)", () => {
    it("upserts into streams with all initial parameters", async () => {
      const client = mockClient();

      await applyEvent(
        client,
        ev({
          type: "stream_created",
          ledger: 10,
          contractId: "CSTREAM0001",
          txHash: "deadbeef",
          fields: {
            sender: "GSENDER",
            recipient: "GRECIPIENT",
            token: "GTOKEN",
            rate_per_second: "1000000",
            start_time: 1700000000,
            end_time: 1700003600,
          },
        }),
      );

      expect(logged).toHaveLength(0); // recognised — no "dropping" warn line
      expect(client.query).toHaveBeenCalledTimes(1);
      const [sql, params] = (client.query as ReturnType<typeof vi.fn>).mock.calls[0] as [
        string,
        unknown[],
      ];

      expect(sql).toContain("INSERT INTO streams");
      expect(sql).toContain(
        "ON CONFLICT (id) DO UPDATE",
      );
      // Columns from the issue's acceptance criteria, in order.
      expect(sql).toContain("id, sender, recipient, token, rate_per_second, start_time, end_time");
      expect(params).toEqual([
        "CSTREAM0001", // id — contract address when no decoder stream_id
        "GSENDER",
        "GRECIPIENT",
        "GTOKEN",
        "1000000",
        1700000000,
        1700003600,
      ]);
    });

    it("prefers a decoder-supplied stream_id over the contract address", async () => {
      const client = mockClient();

      await applyEvent(
        client,
        ev({
          type: "created",
          contractId: "CSTREAM0001",
          fields: {
            stream_id: "42",
            sender: "GSENDER",
            recipient: "GRECIPIENT",
            token: "GTOKEN",
            rate_per_second: "7",
            start_time: 1,
            end_time: 0,
          },
        }),
      );

      const [sql, params] = (client.query as ReturnType<typeof vi.fn>).mock.calls[0] as [
        string,
        unknown[],
      ];
      // `created` is the raw on-chain topic name — accepted as an alias.
      expect(sql).toContain("INSERT INTO streams");
      expect(params[0]).toBe("42");
    });

    it("is idempotent — re-folding the same creation rewrites the same row", async () => {
      const client = mockClient();
      const event = ev({
        type: "stream_created",
        contractId: "CSTREAM0001",
        fields: { sender: "GS", recipient: "GR", token: "GT", rate_per_second: "1" },
      });

      await applyEvent(client, event);
      await applyEvent(client, event);

      expect(client.query).toHaveBeenCalledTimes(2);
      const first = (client.query as ReturnType<typeof vi.fn>).mock.calls[0];
      const second = (client.query as ReturnType<typeof vi.fn>).mock.calls[1];
      expect(first[0]).toBe(second[0]);
      expect(first[1]).toEqual(second[1]);
    });
  });

  describe("stream lifecycle folds (issue #566)", () => {
    function calls(client: PoolClient): [string, unknown[]][] {
      return (client.query as ReturnType<typeof vi.fn>).mock.calls as [string, unknown[]][];
    }

    it("records stream_withdrawn in stream_events with an idempotent upsert", async () => {
      const client = mockClient();

      await applyEvent(
        client,
        ev({
          type: "stream_withdrawn",
          ledger: 123,
          contractId: "CSTREAM0001",
          txHash: "txhash1",
          fields: { amount: "500", total_withdrawn: "1500", remaining: "8500", sequence: 2 },
        }),
      );

      expect(logged).toHaveLength(0);
      expect(client.query).toHaveBeenCalledTimes(1);
      const [sql, params] = calls(client)[0];

      expect(sql).toContain("INSERT INTO stream_events");
      expect(sql).toContain(
        "ON CONFLICT (ledger, tx_hash, event_type, stream_id) DO UPDATE",
      );
      expect(params.slice(0, 5)).toEqual([
        "CSTREAM0001",
        "stream_withdrawn",
        123,
        "txhash1",
        "500",
      ]);
      expect(JSON.parse(params[5] as string)).toEqual({
        amount: "500",
        total_withdrawn: "1500",
        remaining: "8500",
        sequence: 2,
      });
    });

    it("records stream_cancelled with the refunded amount", async () => {
      const client = mockClient();

      await applyEvent(
        client,
        ev({
          type: "stream_cancelled",
          ledger: 130,
          contractId: "CSTREAM0001",
          txHash: "txhash2",
          fields: { refund_amount: "9000", withdrawn_so_far: "1000", sequence: 3 },
        }),
      );

      const [sql, params] = calls(client)[0];
      expect(sql).toContain("INSERT INTO stream_events");
      expect(params.slice(0, 5)).toEqual([
        "CSTREAM0001",
        "stream_cancelled",
        130,
        "txhash2",
        "9000",
      ]);
    });

    it("accepts the raw on-chain topic names (withdrawn / cancelled) as aliases", async () => {
      const client = mockClient();

      await applyEvent(client, ev({ type: "withdrawn", ledger: 1, fields: { amount: "1" } }));
      await applyEvent(client, ev({ type: "cancelled", ledger: 2, fields: { refund_amount: "2" } }));

      const [sql1, params1] = calls(client)[0];
      const [sql2, params2] = calls(client)[1];
      expect(sql1).toContain("INSERT INTO stream_events");
      expect(sql2).toContain("INSERT INTO stream_events");
      // Stored under the canonical README name regardless of the input tag.
      expect(params1[1]).toBe("stream_withdrawn");
      expect(params2[1]).toBe("stream_cancelled");
    });

    it("covers the rest of the README's DripStream events table too", async () => {
      const client = mockClient();

      for (const type of [
        "stream_paused",
        "stream_resumed",
        "stream_topped_up",
        "stream_clawback",
        "xfer_rec",
      ]) {
        await applyEvent(client, ev({ type, ledger: 5, fields: { amount: "1" } }));
      }

      expect(logged).toHaveLength(0);
      expect(client.query).toHaveBeenCalledTimes(5);
      const storedTypes = calls(client).map(([, params]) => params[1]);
      expect(storedTypes).toEqual([
        "stream_paused",
        "stream_resumed",
        "stream_topped_up",
        "stream_clawback",
        "xfer_rec",
      ]);
    });

    it("still logs-and-drops event types nobody has a fold for", async () => {
      const client = mockClient();

      await applyEvent(client, ev({ type: "governor_upgraded", ledger: 7 }));

      expect(client.query).not.toHaveBeenCalled();
      expect(logged).toHaveLength(1);
      expect(lastLogLine().type).toBe("governor_upgraded");
    });
  });
});
