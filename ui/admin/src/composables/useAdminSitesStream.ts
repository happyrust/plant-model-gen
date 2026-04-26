import { onBeforeUnmount, onMounted, ref } from 'vue'

/**
 * D1 / Sprint D · useAdminSitesStream（修 G7/G8）
 *
 * 包装后端 `/api/sync/events` SSE 流，订阅 admin 站点相关事件并提供：
 *   - 自动连接 / 断开 / 指数退避重连
 *   - realtimeConnected / reconnectAttempt 状态（驱动 UI「实时已连接」徽标）
 *   - 三个事件回调：snapshot / created / deleted
 *   - 重连成功回调 onConnect（断流期可能漏事件，由调用方触发一次全量 fetch）
 *
 * 复用同一条 SSE 通道（与 `useCollaborationStream` 共用 `SYNC_EVENT_TX`），
 * 但本 composable 仅关心 admin 站点事件类型；其他事件（如 collab/MQTT）
 * 直接忽略，由其他 composable 各自订阅处理。
 *
 * 与 `useCollaborationStream.ts` 的差异：
 *   - 不带 dev mock（admin 站点的 SSE 验证依赖真后端）
 *   - 回调 + 类型签名按 admin 站点事件域裁剪
 */

export interface AdminSiteSnapshotPayload {
  site_id: string
  project_name?: string | null
  status: string
  parse_status: string
  last_error?: string | null
  timestamp: string
}

export interface AdminSiteCreatedPayload {
  site_id: string
  project_name: string
  timestamp: string
}

export interface AdminSiteDeletedPayload {
  site_id: string
  timestamp: string
}

type AdminSiteSseEvent =
  | { type: 'AdminSiteSnapshot'; data: AdminSiteSnapshotPayload }
  | { type: 'AdminSiteCreated'; data: AdminSiteCreatedPayload }
  | { type: 'AdminSiteDeleted'; data: AdminSiteDeletedPayload }
  | { type: string; data?: unknown }

export interface AdminSitesStreamCallbacks {
  onSnapshot?: (payload: AdminSiteSnapshotPayload) => void
  onCreated?: (payload: AdminSiteCreatedPayload) => void
  onDeleted?: (payload: AdminSiteDeletedPayload) => void
  /**
   * 重连成功（reconnectAttempt > 0 → 0）时触发，调用方可在此 fetchSites
   * 做一次全量同步，弥补断流期间漏掉的事件。
   */
  onConnect?: () => void
}

export interface AdminSitesStreamOptions {
  url?: string
  reconnectInitialMs?: number
  reconnectMaxMs?: number
  autoConnect?: boolean
  callbacks?: AdminSitesStreamCallbacks
}

// 后端 SSE 端点（mod.rs L760 注册）：
//   GET /api/sync/events/stream  → sse_handlers::sync_events_handler
// 注意：`/api/sync/events`（不带 /stream）是 polling list endpoint，
// 形如 `{ events: [], status: "success", timestamp: ... }`，并非 SSE 流。
const DEFAULT_URL = '/api/sync/events/stream'
const DEFAULT_RECONNECT_INITIAL = 1000
const DEFAULT_RECONNECT_MAX = 30000

export function useAdminSitesStream(options: AdminSitesStreamOptions = {}) {
  const {
    url = DEFAULT_URL,
    reconnectInitialMs = DEFAULT_RECONNECT_INITIAL,
    reconnectMaxMs = DEFAULT_RECONNECT_MAX,
    autoConnect = true,
    callbacks = {},
  } = options

  const realtimeConnected = ref(false)
  const reconnectAttempt = ref(0)
  const lastEventAt = ref<string | null>(null)

  let eventSource: EventSource | null = null
  let reconnectTimer: number | null = null

  function clearReconnect() {
    if (reconnectTimer != null) {
      window.clearTimeout(reconnectTimer)
      reconnectTimer = null
    }
  }

  function scheduleReconnect() {
    clearReconnect()
    const delay = Math.min(
      reconnectMaxMs,
      reconnectInitialMs * Math.pow(2, reconnectAttempt.value),
    )
    reconnectAttempt.value += 1
    reconnectTimer = window.setTimeout(() => connect(), delay)
  }

  function disconnect() {
    clearReconnect()
    if (eventSource) {
      eventSource.close()
      eventSource = null
    }
    realtimeConnected.value = false
  }

  function connect() {
    disconnect()

    if (typeof EventSource === 'undefined') {
      return
    }

    try {
      eventSource = new EventSource(url)
    } catch {
      scheduleReconnect()
      return
    }

    eventSource.addEventListener('open', () => {
      const wasReconnecting = reconnectAttempt.value > 0
      realtimeConnected.value = true
      reconnectAttempt.value = 0
      if (wasReconnecting) callbacks.onConnect?.()
    })

    eventSource.addEventListener('message', (event) => {
      lastEventAt.value = new Date().toISOString()
      try {
        const evt = JSON.parse((event as MessageEvent).data) as AdminSiteSseEvent
        switch (evt.type) {
          case 'AdminSiteSnapshot':
            if (evt.data) callbacks.onSnapshot?.(evt.data as AdminSiteSnapshotPayload)
            break
          case 'AdminSiteCreated':
            if (evt.data) callbacks.onCreated?.(evt.data as AdminSiteCreatedPayload)
            break
          case 'AdminSiteDeleted':
            if (evt.data) callbacks.onDeleted?.(evt.data as AdminSiteDeletedPayload)
            break
          default:
            // 其他事件类型（MqttSubscriptionStatusChanged / Started / SyncProgress 等）
            // 由其他 composable 处理，本 composable 直接 ignore
            break
        }
      } catch {
        // ignore malformed payload
      }
    })

    eventSource.addEventListener('error', () => {
      realtimeConnected.value = false
      if (eventSource?.readyState === EventSource.CLOSED) {
        scheduleReconnect()
      }
    })
  }

  if (autoConnect) {
    onMounted(() => connect())
    onBeforeUnmount(() => disconnect())
  }

  return {
    realtimeConnected,
    lastEventAt,
    reconnectAttempt,
    connect,
    disconnect,
  }
}
