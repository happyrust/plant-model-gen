import type { ManagedSiteParsePlan } from '@/types/site'
import { parseDbTypeLabelMap } from './parse-db-types'

export interface ParsePlanFileGroup {
  dbType: string
  label: string
  files: string[]
}

const UNKNOWN_DB_TYPE = 'UNKNOWN'

function normalizeDbType(value: string | null | undefined): string {
  const normalized = value?.trim().toUpperCase()
  return normalized || UNKNOWN_DB_TYPE
}

function dbTypeLabel(dbType: string): string {
  if (dbType === UNKNOWN_DB_TYPE) return '未知类型'
  return parseDbTypeLabelMap[dbType] ?? dbType
}

export function groupParsePlanFilesByDbType(plan: ManagedSiteParsePlan | null | undefined): ParsePlanFileGroup[] {
  if (!plan?.included_db_files?.length) return []

  const entryByFileName = new Map((plan.entries ?? []).map((entry) => [entry.file_name, entry]))
  const groups = new Map<string, ParsePlanFileGroup>()

  for (const file of plan.included_db_files) {
    const dbType = normalizeDbType(entryByFileName.get(file)?.db_type)
    const existing = groups.get(dbType)
    if (existing) {
      existing.files.push(file)
    } else {
      groups.set(dbType, {
        dbType,
        label: dbTypeLabel(dbType),
        files: [file],
      })
    }
  }

  return [...groups.values()]
}
