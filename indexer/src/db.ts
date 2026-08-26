import { Pool } from 'pg'

export function createPool(): Pool {
  const connectionString = process.env.DATABASE_URL
  if (!connectionString) {
    throw new Error('DATABASE_URL is not set')
  }
  return new Pool({ connectionString })
}
