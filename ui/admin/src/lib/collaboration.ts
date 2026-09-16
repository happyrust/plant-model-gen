import type {
  CollaborationRuntimeStatus,
  CollaborationSiteAvailability,
  CollaborationTone,
} from '@/types/collaboration'

function padDatePart(value: number) {
  return String(value).padStart(2, '0')
}

function normalizeDate(value?: string | null) {
  if (!value) return null
  const date = new Date(value)
  if (Number.isNaN(date.getTime())) return null
  return date
}

export function formatDateTime(value?: string | null) {
  const date = normalizeDate(value)
  if (!date) return '-'
  return `${date.getFullYear()}-${padDatePart(date.getMonth() + 1)}-${padDatePart(date.getDate())} ${padDatePart(date.getHours())}:${padDatePart(date.getMinutes())}`
}

export function formatRelativeTime(value?: string | null) {
  const date = normalizeDate(value)
  if (!date) return '未知'

  const diff = Date.now() - date.getTime()
  const minute = 60 * 1000
  const hour = 60 * minute
  const day = 24 * hour

  if (diff < minute) return '刚刚更新'
  if (diff < hour) return `${Math.max(1, Math.floor(diff / minute))} 分钟前`
  if (diff < day) return `${Math.max(1, Math.floor(diff / hour))} 小时前`
  return `${Math.max(1, Math.floor(diff / day))} 天前`
}

export function formatBytes(value?: number | null) {
  if (!value || value <= 0) return '0 B'
  const units = ['B', 'KB', 'MB', 'GB', 'TB']
  let size = value
  let unitIndex = 0

  while (size >= 1024 && unitIndex < units.length - 1) {
    size /= 1024
    unitIndex += 1
  }

  return `${size >= 10 || unitIndex === 0 ? size.toFixed(0) : size.toFixed(1)} ${units[unitIndex]}`
}

export function getRuntimeLabel(envId: string, runtimeStatus: CollaborationRuntimeStatus | null) {
  if (!runtimeStatus?.active || runtimeStatus.env_id !== envId) {
    return '未激活'
  }
  if (runtimeStatus.mqtt_connected === false) {
    return 'MQTT 未连接'
  }
  return '已激活'
}

export function getRuntimeTone(label: string): CollaborationTone {
  if (label === '已激活') return 'success'
  if (label === 'MQTT 未连接') return 'warning'
  return 'default'
}

export function getAvailabilityTone(availability: CollaborationSiteAvailability): CollaborationTone {
  if (availability === 'online') return 'success'
  if (availability === 'cached') return 'warning'
  return 'default'
}
