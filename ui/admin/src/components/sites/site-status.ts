import type {
  ManagedProjectSite,
  ManagedSiteParsePlan,
  ManagedSiteParseStatus,
  ManagedSiteStatus,
} from '@/types/site'

export type QuickFilter = 'all' | 'running' | 'busy' | 'error' | 'pending_parse'

export const statusLabelMap: Record<ManagedSiteStatus, string> = {
  Draft: '草稿',
  Parsed: '已解析',
  Starting: '启动中',
  Running: '运行中',
  Stopping: '停止中',
  Stopped: '已停止',
  Failed: '失败',
}

export const statusClassMap: Record<ManagedSiteStatus, string> = {
  Draft: 'bg-muted text-muted-foreground',
  Parsed: 'bg-blue-100 text-blue-800 dark:bg-blue-900 dark:text-blue-200',
  Starting: 'bg-amber-100 text-amber-800 dark:bg-amber-900 dark:text-amber-200',
  Running: 'bg-green-100 text-green-800 dark:bg-green-900 dark:text-green-200',
  Stopping: 'bg-amber-100 text-amber-800 dark:bg-amber-900 dark:text-amber-200',
  Stopped: 'bg-muted text-muted-foreground',
  Failed: 'bg-red-100 text-red-800 dark:bg-red-900 dark:text-red-200',
}

export const parseStatusLabelMap: Record<ManagedSiteParseStatus, string> = {
  Pending: '待解析',
  Running: '解析中',
  Parsed: '已解析',
  Failed: '解析失败',
}

export function parseStatusClass(status: ManagedSiteParseStatus): string {
  if (status === 'Parsed') return 'bg-green-100 text-green-800 dark:bg-green-900 dark:text-green-200'
  if (status === 'Running') return 'bg-blue-100 text-blue-800 dark:bg-blue-900 dark:text-blue-200'
  if (status === 'Failed') return 'bg-red-100 text-red-800 dark:bg-red-900 dark:text-red-200'
  return 'bg-muted text-muted-foreground'
}

export function parsePlanClass(plan?: ManagedSiteParsePlan | null): string {
  if (plan?.mode === 'FastReparse') {
    return 'bg-cyan-100 text-cyan-800 dark:bg-cyan-900 dark:text-cyan-200'
  }
  if (plan?.mode === 'RebuildSystem') {
    return 'bg-amber-100 text-amber-800 dark:bg-amber-900 dark:text-amber-200'
  }
  if (plan?.mode === 'Selective') {
    return 'bg-blue-100 text-blue-800 dark:bg-blue-900 dark:text-blue-200'
  }
  if (plan?.mode === 'Bootstrap') {
    return 'bg-violet-100 text-violet-800 dark:bg-violet-900 dark:text-violet-200'
  }
  return 'bg-slate-100 text-slate-700 dark:bg-slate-800 dark:text-slate-200'
}

export function isSiteBusy(site: ManagedProjectSite): boolean {
  return (
    site.status === 'Starting' ||
    site.status === 'Stopping' ||
    site.parse_status === 'Running'
  )
}

export function canStartSite(site: ManagedProjectSite): boolean {
  return (
    site.parse_status !== 'Running' &&
    !isSiteBusy(site) &&
    ['Stopped', 'Parsed', 'Failed', 'Draft'].includes(site.status)
  )
}

export function canStopSite(site: ManagedProjectSite): boolean {
  return (
    site.status === 'Running' ||
    site.status === 'Starting' ||
    site.parse_status === 'Running'
  )
}

export function canParseSite(site: ManagedProjectSite): boolean {
  return (
    !isSiteBusy(site) &&
    site.status !== 'Running'
  )
}

export function canDeleteSite(site: ManagedProjectSite): boolean {
  return (
    !isSiteBusy(site) &&
    site.status !== 'Running'
  )
}

export function canEditSite(site: ManagedProjectSite): boolean {
  return canDeleteSite(site)
}

export function isSiteError(site: ManagedProjectSite): boolean {
  return site.status === 'Failed' || !!site.last_error
}

export function isSiteRunning(site: ManagedProjectSite): boolean {
  return site.status === 'Running'
}

export function isPendingParse(site: ManagedProjectSite): boolean {
  return (
    site.parse_status === 'Pending' &&
    site.status !== 'Running' &&
    site.status !== 'Starting'
  )
}

export function matchesQuickFilter(site: ManagedProjectSite, filter: QuickFilter): boolean {
  switch (filter) {
    case 'all': return true
    case 'running': return isSiteRunning(site)
    case 'busy': return isSiteBusy(site)
    case 'error': return isSiteError(site)
    case 'pending_parse': return isPendingParse(site)
  }
}

export const quickFilterOptions: { value: QuickFilter; label: string }[] = [
  { value: 'all', label: '全部' },
  { value: 'running', label: '运行中' },
  { value: 'busy', label: '处理中' },
  { value: 'error', label: '异常' },
  { value: 'pending_parse', label: '待解析' },
]

/**
 * 动作的中文短标签，供错误提示里拼接（例如 "解析失败：..."）使用。
 *
 * 与 `stores/sites.ts::SiteAction` 一一对应；新增动作时两处保持同步。
 */
export const siteActionLabelMap = {
  parse: '解析',
  start: '启动',
  stop: '停止',
  delete: '删除',
} as const satisfies Record<'parse' | 'start' | 'stop' | 'delete', string>

export interface SiteStatsExtended {
  total: number
  running: number
  busy: number
  error: number
  pending_parse: number
}

export function computeStats(sites: ManagedProjectSite[]): SiteStatsExtended {
  let running = 0
  let busy = 0
  let error = 0
  let pending_parse = 0
  for (const site of sites) {
    if (isSiteRunning(site)) running++
    if (isSiteBusy(site)) busy++
    if (isSiteError(site)) error++
    if (isPendingParse(site)) pending_parse++
  }
  return { total: sites.length, running, busy, error, pending_parse }
}
