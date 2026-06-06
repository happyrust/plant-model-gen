import { sitesApi } from '@/api/sites'
import type { ManagedSitePreflightReport } from '@/types/site'

export interface PortConflictItem {
  label: string
  port: number
  pids: number[]
}

export interface PortConflictGuardResult {
  ok: boolean
  message?: string
}

export async function checkPortConflicts(
  ports: Array<{ label: string; port?: number | null }>,
  host?: string,
) {
  const conflicts: PortConflictItem[] = []
  const seenPorts = new Set<number>()
  for (const item of ports) {
    const port = Number(item.port)
    if (!Number.isInteger(port) || port <= 0 || port > 65535 || seenPorts.has(port)) continue
    seenPorts.add(port)
    const result = await sitesApi.checkPort(port, host)
    if (result.in_use) {
      conflicts.push({ label: item.label, port, pids: result.pids })
    }
  }
  return conflicts
}

export function preflightPortConflicts(report: ManagedSitePreflightReport | null | undefined) {
  if (!report) return []
  return report.checks
    .filter((check) => check.status === 'blocking' && check.pids.length > 0 && check.message.includes('端口'))
    .map((check) => {
      const match = check.message.match(/端口\s*(\d+)/)
      return {
        label: check.label,
        port: match ? Number(match[1]) : 0,
        pids: check.pids,
      }
    })
    .filter((item) => Number.isInteger(item.port) && item.port > 0)
}

export async function resolvePortConflicts(
  conflicts: PortConflictItem[],
  context: string,
): Promise<PortConflictGuardResult> {
  if (conflicts.length === 0) return { ok: true }

  const lines = conflicts
    .map((item) => `${item.label}端口 ${item.port} 被占用，PIDs: ${item.pids.join(', ') || '-'}`)
    .join('\n')
  const confirmed = window.confirm(
    `${context}\n\n检测到端口冲突：\n${lines}\n\n是否结束占用这些端口的外部进程，然后继续？`,
  )
  if (!confirmed) {
    return { ok: false, message: '已取消：检测到端口冲突，未结束占用进程，因此没有继续执行。' }
  }

  const remaining: PortConflictItem[] = []
  for (const conflict of conflicts) {
    const result = await sitesApi.killPort(conflict.port)
    if (!result.released) {
      remaining.push({
        ...conflict,
        pids: result.remaining_pids,
      })
    }
  }

  if (remaining.length === 0) return { ok: true }

  const remainingText = remaining
    .map((item) => `${item.label}端口 ${item.port} 仍被占用，PIDs: ${item.pids.join(', ') || '-'}`)
    .join('\n')
  return {
    ok: false,
    message: `端口仍未释放，已停止后续动作：\n${remainingText}`,
  }
}
