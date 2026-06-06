import { Surreal } from 'surrealdb'
import type {
  DataBrowserConnectionResponse,
  DataBrowserRecordsResponse,
  DataBrowserRecordParams,
  DataBrowserTable,
} from '@/api/data-browser'

const WRITE_KEYWORDS = new Set([
  'CREATE',
  'UPDATE',
  'DELETE',
  'INSERT',
  'UPSERT',
  'RELATE',
  'REMOVE',
  'DEFINE',
])

export type SurrealQueryResponse =
  | { success: true; result: unknown; type?: string; stats?: unknown }
  | { success: false; error: unknown; stats?: unknown }

export function isLoopbackHost(hostname = window.location.hostname): boolean {
  const normalized = hostname.trim().toLowerCase().replace(/^\[|\]$/g, '')
  return normalized === 'localhost' || normalized === '127.0.0.1' || normalized === '::1'
}

export function isSafeSurrealIdent(value: string): boolean {
  return /^[A-Za-z_][A-Za-z0-9_]*$/.test(value.trim())
}

export function requireSafeSurrealIdent(value: string, label: string): string {
  const trimmed = value.trim()
  if (!isSafeSurrealIdent(trimmed)) {
    throw new Error(`${label} 格式不正确，仅支持字母、数字、下划线且不能以数字开头: ${trimmed}`)
  }
  return trimmed
}

export function isWriteSurrealQuery(sql: string): boolean {
  const withoutComments = sql
    .replace(/--.*$/gm, ' ')
    .replace(/\/\*[\s\S]*?\*\//g, ' ')
  const statements = withoutComments
    .split(';')
    .map((statement) => statement.trim())
    .filter(Boolean)
  return statements.some((statement) => {
    const [keyword = ''] = statement.split(/\s+/, 1)
    return WRITE_KEYWORDS.has(keyword.toUpperCase())
  })
}

function collectColumns(records: Record<string, unknown>[]): string[] {
  const keys = new Set<string>()
  for (const record of records) {
    for (const key of Object.keys(record)) keys.add(key)
  }
  const columns = Array.from(keys).sort((a, b) => a.localeCompare(b))
  if (keys.has('id')) {
    return ['id', ...columns.filter((column) => column !== 'id')]
  }
  return columns
}

function extractInfoTables(value: unknown): DataBrowserTable[] {
  const info = Array.isArray(value) ? value[0] : value
  const tables = info && typeof info === 'object'
    ? (info as { tables?: unknown }).tables
    : undefined
  if (!tables || typeof tables !== 'object') return []
  return Object.keys(tables)
    .filter(isSafeSurrealIdent)
    .sort((a, b) => a.localeCompare(b))
    .map((name) => ({ name }))
}

function extractTotal(value: unknown): number {
  const rows = Array.isArray(value) ? value : []
  const total = rows[0] && typeof rows[0] === 'object'
    ? (rows[0] as { total?: unknown }).total
    : undefined
  return typeof total === 'number' && Number.isFinite(total) ? total : 0
}

function normalizeRecords(value: unknown): Record<string, unknown>[] {
  if (!Array.isArray(value)) return []
  return value.filter((record): record is Record<string, unknown> =>
    Boolean(record && typeof record === 'object' && !Array.isArray(record)),
  )
}

export class SiteSurrealBrowserClient {
  private db: Surreal | null = null
  private connection: DataBrowserConnectionResponse | null = null

  get isConnected(): boolean {
    return Boolean(this.db?.isConnected)
  }

  get context(): DataBrowserConnectionResponse | null {
    return this.connection
  }

  async connect(connection: DataBrowserConnectionResponse): Promise<void> {
    await this.close()
    const db = new Surreal()
    await db.connect(connection.endpoint)
    await db.signin({
      namespace: connection.namespace,
      database: connection.database,
      username: connection.credential.username,
      password: connection.credential.password,
    })
    await db.use({
      namespace: connection.namespace,
      database: connection.database,
    })
    this.db = db
    this.connection = connection
  }

  async close(): Promise<void> {
    const db = this.db
    this.db = null
    this.connection = null
    if (db?.isConnected) await db.close()
  }

  async listTables(): Promise<DataBrowserTable[]> {
    const [info] = await this.queryValues<[unknown]>('INFO FOR DB;')
    return extractInfoTables(info)
  }

  async fetchRecords(tableName: string, params: DataBrowserRecordParams = {}): Promise<DataBrowserRecordsResponse> {
    const table = requireSafeSurrealIdent(tableName, 'table')
    const page = Math.max(1, params.page ?? 1)
    const perPage = Math.min(100, Math.max(1, params.per_page ?? 25))
    const start = (page - 1) * perPage
    const sort = requireSafeSurrealIdent(params.sort?.trim() || 'id', 'sort')
    const dir = params.dir === 'desc' ? 'desc' : 'asc'
    const [recordsValue, countValue] = await this.queryValues<[unknown, unknown]>(
      `SELECT *, id AS id FROM ${table} ORDER BY ${sort} ${dir.toUpperCase()} LIMIT ${perPage} START ${start};
SELECT count() AS total FROM ${table} GROUP ALL;`,
    )
    const records = normalizeRecords(recordsValue)
    return {
      table,
      columns: collectColumns(records),
      records,
      total: extractTotal(countValue),
      page,
      per_page: perPage,
      sort,
      dir,
      site: this.connection?.site,
    }
  }

  async runQuery(sql: string): Promise<SurrealQueryResponse[]> {
    return this.queryResponses(sql)
  }

  private getDb(): Surreal {
    if (!this.db?.isConnected) throw new Error('SurrealDB 尚未连接')
    return this.db
  }

  private async queryValues<T extends unknown[]>(sql: string): Promise<T> {
    return await this.getDb().query(sql).json().collect() as T
  }

  private async queryResponses(sql: string): Promise<SurrealQueryResponse[]> {
    return await this.getDb().query(sql).json().responses() as SurrealQueryResponse[]
  }
}
