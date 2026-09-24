import { createPool } from './db.js'
import { SorobanEventSource } from './indexer/chainEventSource.js'
import { Poller } from './indexer/poller.js'

async function main(): Promise<void> {
  const pool = createPool()
  const poller = new Poller({
    pool,
    source: new SorobanEventSource(),
    pageSize: Number(process.env.PAGE_SIZE ?? 100),
    pollIntervalMs: Number(process.env.POLL_INTERVAL_MS ?? 2000),
  })

  let shuttingDown = false
  const shutdown = async (signal: string): Promise<void> => {
    if (shuttingDown) return
    shuttingDown = true
    console.log(`[worker] received ${signal}, shutting down`)
    poller.stop()
    await pool.end()
    process.exit(0)
  }

  process.on('SIGINT', () => void shutdown('SIGINT'))
  process.on('SIGTERM', () => void shutdown('SIGTERM'))

  console.log('[worker] starting poller')
  await poller.start()
}

main().catch((err) => {
  console.error('[worker] fatal error', err)
  process.exit(1)
})
