import { describe, expect, it } from 'vitest'
import { readCursor, saveCursor, type Cursor } from '../src/indexer/poller.js'

/**
 * Minimal fake standing in for a pg Pool, just enough to exercise cursor read/save.
 * Deliberately narrow in scope for this scaffold — see the top-level README's
 * "Known gaps" section for the poller behaviors (cursor persistence ordering,
 * resume-after-restart, page-boundary re-delivery) that still need real coverage
 * against a live Postgres instance.
 */
function fakePool(initial: Cursor) {
  let row = { token: initial.token, last_ledger: String(initial.lastLedger) }
  return {
    async query(sql: string, params?: unknown[]) {
      if (sql.startsWith('SELECT')) {
        return { rows: [row] }
      }
      if (sql.startsWith('UPDATE')) {
        const [token, lastLedger] = params as [string | null, number]
        row = { token, last_ledger: String(lastLedger) }
        return { rows: [] }
      }
      throw new Error(`unexpected query: ${sql}`)
    },
  }
}

describe('cursor persistence', () => {
  it('reads back what was saved', async () => {
    const pool = fakePool({ token: null, lastLedger: 0 })

    await saveCursor(pool as never, { token: 'abc', lastLedger: 42 })
    const cursor = await readCursor(pool as never)

    expect(cursor).toEqual({ token: 'abc', lastLedger: 42 })
  })
})
