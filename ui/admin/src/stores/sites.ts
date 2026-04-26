import { defineStore } from 'pinia'
import { ref, computed, reactive } from 'vue'
import { extractErrorMessage } from '@/api/client'
import { sitesApi } from '@/api/sites'
import type {
  ManagedProjectSite,
  SiteStats,
  CreateManagedSiteRequest,
  UpdateManagedSiteRequest,
  ManagedSiteStatus,
  ManagedSiteParseStatus,
} from '@/types/site'
import type {
  AdminSiteSnapshotPayload,
  AdminSiteCreatedPayload,
  AdminSiteDeletedPayload,
} from '@/composables/useAdminSitesStream'

export type SiteAction = 'parse' | 'start' | 'stop' | 'restart' | 'delete'
export interface SiteActionError {
  siteId: string
  action: SiteAction
  message: string
}

// D3 / Sprint D · 批量操作类型
export type SiteBulkAction = 'start' | 'stop' | 'restart' | 'parse' | 'delete'

export interface SiteBulkResult {
  total: number
  ok: number
  failed: { siteId: string; message: string }[]
}

export const useSitesStore = defineStore('sites', () => {
  const sites = ref<ManagedProjectSite[]>([])
  const loading = ref(false)
  const error = ref('')
  const pendingActions = reactive(new Map<string, SiteAction>())
  const actionErrors = reactive(new Map<string, SiteActionError>())
  const latestActionError = ref<SiteActionError | null>(null)

  const stats = computed<SiteStats>(() => ({
    total: sites.value.length,
    running: sites.value.filter((s) => s.status === 'Running').length,
    error: sites.value.filter((s) => s.status === 'Failed').length,
    pending_parse: sites.value.filter((s) => s.parse_status === 'Pending').length,
  }))

  function getSiteAction(siteId: string): SiteAction | undefined {
    return pendingActions.get(siteId)
  }

  function isSiteActionPending(siteId: string): boolean {
    return pendingActions.has(siteId)
  }

  function getSiteActionError(siteId: string): SiteActionError | undefined {
    return actionErrors.get(siteId)
  }

  function clearSiteActionError(siteId: string) {
    actionErrors.delete(siteId)
    if (latestActionError.value?.siteId === siteId) {
      latestActionError.value = null
    }
  }

  async function withAction(siteId: string, action: SiteAction, fn: () => Promise<void>) {
    if (pendingActions.has(siteId)) return
    pendingActions.set(siteId, action)
    clearSiteActionError(siteId)
    try {
      await fn()
    } catch (err: unknown) {
      const message = extractErrorMessage(err)
      const payload = { siteId, action, message }
      actionErrors.set(siteId, payload)
      latestActionError.value = payload
      throw err
    } finally {
      pendingActions.delete(siteId)
    }
  }

  async function fetchSites() {
    loading.value = true
    error.value = ''
    try {
      sites.value = await sitesApi.list()
    } catch (err: unknown) {
      error.value = err instanceof Error ? err.message : 'Failed to fetch sites'
    } finally {
      loading.value = false
    }
  }

  async function createSite(payload: CreateManagedSiteRequest) {
    const site = await sitesApi.create(payload)
    sites.value.push(site)
    return site
  }

  async function updateSite(id: string, payload: UpdateManagedSiteRequest) {
    const updated = await sitesApi.update(id, payload)
    const idx = sites.value.findIndex((s) => s.site_id === id)
    if (idx !== -1) sites.value[idx] = updated
    return updated
  }

  async function deleteSite(id: string) {
    await withAction(id, 'delete', async () => {
      await sitesApi.delete(id)
      sites.value = sites.value.filter((s) => s.site_id !== id)
    })
  }

  // ─── D1 / Sprint D · SSE patcher（修 G7/G8） ─────────────────────────────
  //
  // 前端订阅 `/api/sync/events` 后，把 admin 站点事件按 site_id patch 到
  // 本地 `sites.value` 中，避免每次状态变更都全量 `fetchSites()`。
  //
  // 设计取舍：
  // - `AdminSiteSnapshot` → 仅 patch 已知字段（status/parse_status/last_error/project_name），
  //   事件未携带的字段（runtime_dir / risk_level / created_at 等）保留旧值；
  // - `AdminSiteCreated` → payload 仅 site_id + project_name，缺少 db_port / status 等
  //   完整字段，调用方应直接 fetchSites() 拉完整列表（一次性轻量 GET），
  //   本 patcher 不构造不完整的 site 对象避免 UI 闪烁；
  // - `AdminSiteDeleted` → 直接 filter 掉，幂等。
  //
  // 重连成功后调用方应调一次 fetchSites() 兜底，弥补断流期间漏掉的事件。

  /**
   * 按 site_id patch 现有 site 的局部字段（D1 SSE handler）
   *
   * 命中：仅当 site 已存在于 `sites.value` 时 patch；
   * 未命中：silent ignore（极少出现，通常说明 SSE 在 fetchSites 之前到达，
   * 下次 fetchSites 自然会补上）。
   */
  function patchSiteSnapshot(payload: AdminSiteSnapshotPayload) {
    const idx = sites.value.findIndex((s) => s.site_id === payload.site_id)
    if (idx === -1) return
    const current = sites.value[idx]
    sites.value.splice(idx, 1, {
      ...current,
      project_name: payload.project_name ?? current.project_name,
      status: payload.status as ManagedSiteStatus,
      parse_status: payload.parse_status as ManagedSiteParseStatus,
      last_error: payload.last_error ?? null,
    })
  }

  /**
   * `AdminSiteCreated` 事件处理（D1 SSE handler）
   *
   * payload 字段不足以构造完整 site，触发轻量级全量 fetchSites。
   * 多次连续 created（批量创建）会去抖到一次（fetchSites 内部已有 loading 标志）。
   */
  async function handleSiteCreated(_payload: AdminSiteCreatedPayload) {
    await fetchSites()
  }

  /**
   * `AdminSiteDeleted` 事件处理（D1 SSE handler）
   */
  function handleSiteDeleted(payload: AdminSiteDeletedPayload) {
    sites.value = sites.value.filter((s) => s.site_id !== payload.site_id)
  }

  /**
   * SSE 重连成功兜底（D1 SSE handler）
   *
   * 断流期间可能漏掉的事件由这一次全量 fetchSites 补回；幂等。
   */
  async function refreshOnReconnect() {
    await fetchSites()
  }

  async function parseSite(id: string) {
    await withAction(id, 'parse', async () => {
      await sitesApi.parse(id)
      await fetchSites()
    })
  }

  async function startSite(id: string) {
    await withAction(id, 'start', async () => {
      await sitesApi.start(id)
      await fetchSites()
    })
  }

  async function stopSite(id: string) {
    await withAction(id, 'stop', async () => {
      await sitesApi.stop(id)
      await fetchSites()
    })
  }

  async function restartSite(id: string) {
    await withAction(id, 'restart', async () => {
      await sitesApi.restart(id)
      await fetchSites()
    })
  }

  /**
   * D3 / Sprint D · 批量操作
   *
   * 串行调用每个 site 的对应单条 action，避免一次过载（站点启动占用机器资源
   * 较多，并发会撞 CPU/内存阈值）。返回 `{ ok, failed: [{siteId, message}] }`。
   *
   * 各 site 的 pending 状态由 withAction 单独管理，UI 上仍按行显示 spinner。
   */
  async function bulkAction(siteIds: string[], action: SiteBulkAction): Promise<SiteBulkResult> {
    const result: SiteBulkResult = { total: siteIds.length, ok: 0, failed: [] }
    for (const siteId of siteIds) {
      try {
        switch (action) {
          case 'start':
            await startSite(siteId)
            break
          case 'stop':
            await stopSite(siteId)
            break
          case 'restart':
            await restartSite(siteId)
            break
          case 'parse':
            await parseSite(siteId)
            break
          case 'delete':
            await deleteSite(siteId)
            break
        }
        result.ok += 1
      } catch (err: unknown) {
        result.failed.push({
          siteId,
          message: err instanceof Error ? err.message : '未知错误',
        })
      }
    }
    return result
  }

  return {
    sites, stats, loading, error,
    pendingActions, actionErrors, latestActionError,
    getSiteAction, isSiteActionPending, getSiteActionError, clearSiteActionError,
    fetchSites, createSite, updateSite, deleteSite,
    parseSite, startSite, stopSite, restartSite,
    bulkAction,
    patchSiteSnapshot, handleSiteCreated, handleSiteDeleted, refreshOnReconnect,
  }
})
