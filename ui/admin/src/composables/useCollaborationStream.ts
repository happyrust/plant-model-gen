import { onBeforeUnmount, onMounted, ref } from "vue"
import type {
  CollaborationActiveTask,
  CollaborationConfig,
  CollaborationFailedTask,
  CollaborationStreamEvent,
} from "@/types/collaboration"

/**
 * ROADMAP · M3 · useCollaborationStream
 *
 * 包装 `/api/remote-sync/events/stream` (SSE)，提供：
 *   - 自动连接 / 断开 / 指数退避重连
 *   - realtimeConnected 状态（驱动 UI 的 ONLINE 徽标）
 *   - 事件分发回调（活跃任务、失败任务、站点状态变化、同步完成/失败）
 *
 * 开发模式下（`import.meta.env.DEV` 且 `?mock=sse` 或 `?dev=1`）启用本地 mock：
 *   - 每 4 秒推送一个 `active_task_update`
 *   - 每 30 秒推送一个 `failed_task_new`
 *   - 模拟断线重连 1 次（60 秒后）
 */

export interface CollaborationStreamCallbacks {
  onActiveTask?: (task: CollaborationActiveTask) => void
  onFailedTask?: (task: CollaborationFailedTask) => void
  onSiteStatusChange?: (payload: Extract<CollaborationStreamEvent, { type: "site_status_change" }>) => void
  onSyncCompleted?: (payload: Extract<CollaborationStreamEvent, { type: "sync_completed" }>) => void
  onSyncFailed?: (payload: Extract<CollaborationStreamEvent, { type: "sync_failed" }>) => void
}

export interface CollaborationStreamOptions {
  url?: string
  config?: Pick<CollaborationConfig, "reconnect_initial_ms" | "reconnect_max_ms">
  autoConnect?: boolean
  mockInDev?: boolean
  callbacks?: CollaborationStreamCallbacks
}

const DEFAULT_STREAM_URL = "/api/remote-sync/events/stream"
const DEFAULT_RECONNECT_INITIAL = 1000
const DEFAULT_RECONNECT_MAX = 30000

function isDevEnabled() {
  if (typeof import.meta !== "undefined" && (import.meta as { env?: { DEV?: boolean } }).env?.DEV) {
    return true
  }
  if (typeof location === "undefined") return false
  return location.search.includes("dev=1") || location.search.includes("mock=sse")
}

export function useCollaborationStream(options: CollaborationStreamOptions = {}) {
  const {
    url = DEFAULT_STREAM_URL,
    config,
    autoConnect = true,
    mockInDev = true,
    callbacks = {},
  } = options

  const realtimeConnected = ref(false)
  const lastEventAt = ref<string | null>(null)
  const reconnectAttempt = ref(0)

  let eventSource: EventSource | null = null
  let reconnectTimer: number | null = null
  let mockTimer: number | null = null

  function dispatchEvent(evt: CollaborationStreamEvent) {
    lastEventAt.value = new Date().toISOString()
    switch (evt.type) {
      case "active_task_update":
        callbacks.onActiveTask?.(evt.task)
        break
      case "failed_task_new":
        callbacks.onFailedTask?.(evt.task)
        break
      case "site_status_change":
        callbacks.onSiteStatusChange?.(evt)
        break
      case "sync_completed":
        callbacks.onSyncCompleted?.(evt)
        break
      case "sync_failed":
        callbacks.onSyncFailed?.(evt)
        break
      case "keepalive":
        break
    }
  }

  function clearReconnect() {
    if (reconnectTimer != null) {
      window.clearTimeout(reconnectTimer)
      reconnectTimer = null
    }
  }

  function scheduleReconnect() {
    clearReconnect()
    const initial = config?.reconnect_initial_ms ?? DEFAULT_RECONNECT_INITIAL
    const max = config?.reconnect_max_ms ?? DEFAULT_RECONNECT_MAX
    const delay = Math.min(max, initial * Math.pow(2, reconnectAttempt.value))
    reconnectAttempt.value += 1
    reconnectTimer = window.setTimeout(() => connect(), delay)
  }

  function disconnect() {
    clearReconnect()
    if (mockTimer != null) {
      window.clearInterval(mockTimer)
      mockTimer = null
    }
    if (eventSource) {
      eventSource.close()
      eventSource = null
    }
    realtimeConnected.value = false
  }

  function connect() {
    disconnect()

    if (mockInDev && isDevEnabled()) {
      startMockStream()
      return
    }

    if (typeof EventSource === "undefined") {
      return
    }

    try {
      eventSource = new EventSource(url)
    } catch {
      scheduleReconnect()
      return
    }

    eventSource.addEventListener("open", () => {
      realtimeConnected.value = true
      reconnectAttempt.value = 0
    })

    eventSource.addEventListener("message", (event) => {
      try {
        const data = JSON.parse((event as MessageEvent).data) as CollaborationStreamEvent
        dispatchEvent(data)
      } catch {
        // ignore malformed payload
      }
    })

    eventSource.addEventListener("error", () => {
      realtimeConnected.value = false
      if (eventSource?.readyState === EventSource.CLOSED) {
        scheduleReconnect()
      }
    })
  }

  // ─────────────── Mock stream（开发时 UI 验证用） ───────────────

  function startMockStream() {
    realtimeConnected.value = true
    reconnectAttempt.value = 0

    let tick = 0
    mockTimer = window.setInterval(() => {
      tick += 1
      const progress = (tick * 8) % 100
      dispatchEvent({
        type: "active_task_update",
        task: {
          task_id: `mock-T-${8800 + tick}`,
          site_id: "site_mock",
          site_name: "Mock 北京站",
          task_name: `增量推送 push · batch ${tick}`,
          file_path: `/inc/2026-04-21/delta-${String(tick).padStart(4, "0")}.surql`,
          progress,
          status: progress >= 100 ? "Completed" : "Running",
        },
      })
      if (tick % 8 === 0) {
        dispatchEvent({
          type: "failed_task_new",
          task: {
            id: `mock-F-${200 + tick}`,
            task_type: "IncrementUpdate",
            site: "Mock 广州站",
            error: `HTTP 500 · file server internal (#${tick})`,
            retry_count: 1,
            max_retries: 5,
            first_failed_at: new Date().toISOString(),
            next_retry_at: new Date(Date.now() + 60_000).toISOString(),
          },
        })
      }
      if (tick % 15 === 0) {
        dispatchEvent({ type: "sync_completed", site_id: "site_mock", file_count: 12, message: "批次同步完成" })
      }
    }, 4000)
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
