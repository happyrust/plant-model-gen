import { rawApi } from "./client"
import type {
  CollaborationActionResult,
  CollaborationDailyStat,
  CollaborationDiagnosticResponse,
  CollaborationEnv,
  CollaborationFlowStat,
  CollaborationLogsResult,
  CollaborationRuntimeConfig,
  CollaborationRuntimeStatus,
  CollaborationSite,
  CollaborationSiteMetadataResponse,
  CreateCollaborationEnvRequest,
  CreateCollaborationSiteRequest,
  UpdateCollaborationEnvRequest,
  UpdateCollaborationSiteRequest,
} from "@/types/collaboration"

interface LegacyStatusResponse {
  status: string
  message?: string
}

interface LegacyItemResponse<T> extends LegacyStatusResponse {
  item?: T
}

interface LegacyListResponse<T> extends LegacyStatusResponse {
  items?: T[]
  total?: number
  limit?: number
  offset?: number
}

interface RuntimeConfigResponse extends LegacyStatusResponse {
  source?: string | null
  config?: {
    mqtt_host?: string | null
    mqtt_port?: number | null
    file_server_host?: string | null
    location?: string | null
    location_dbs?: number[] | null
    sync_live?: boolean
  }
}

interface ActivateEnvResponse extends LegacyStatusResponse {
  env_id?: string
}

interface CreateResponse extends LegacyStatusResponse {
  id?: string
}

function assertSuccess<T extends LegacyStatusResponse>(response: T, fallbackMessage: string): T {
  if (response.status === "success") {
    return response
  }
  throw new Error(response.message || fallbackMessage)
}

function normalizeRuntimeConfig(config?: RuntimeConfigResponse["config"]): CollaborationRuntimeConfig {
  return {
    mqtt_host: config?.mqtt_host ?? null,
    mqtt_port: config?.mqtt_port ?? null,
    file_server_host: config?.file_server_host ?? null,
    location: config?.location ?? null,
    location_dbs: Array.isArray(config?.location_dbs)
      ? config.location_dbs.filter((value): value is number => typeof value === "number")
      : [],
    sync_live: config?.sync_live ?? false,
    source: null,
  }
}

export const collaborationApi = {
  async listEnvs() {
    const response = await rawApi<LegacyListResponse<CollaborationEnv>>("/api/remote-sync/envs", {
      method: "GET",
    })
    return assertSuccess(response, "加载协同组失败").items ?? []
  },

  async getEnv(id: string) {
    const response = await rawApi<LegacyItemResponse<CollaborationEnv>>("/api/remote-sync/envs/" + id, {
      method: "GET",
    })
    return assertSuccess(response, "加载协同组详情失败").item ?? null
  },

  async listSites(envId: string) {
    const response = await rawApi<LegacyListResponse<CollaborationSite>>("/api/remote-sync/envs/" + envId + "/sites", {
      method: "GET",
    })
    return assertSuccess(response, "加载协同站点失败").items ?? []
  },

  async getRuntimeStatus() {
    const response = await rawApi<CollaborationRuntimeStatus>("/api/remote-sync/runtime/status", {
      method: "GET",
    })
    return assertSuccess(response, "加载运行时状态失败")
  },

  async getRuntimeConfig() {
    const response = await rawApi<RuntimeConfigResponse>("/api/remote-sync/runtime/config", {
      method: "GET",
    })
    const result = assertSuccess(response, "加载运行时配置失败")
    const config = normalizeRuntimeConfig(result.config)
    return {
      ...config,
      source: result.source ?? null,
    }
  },

  async listLogs(params: Record<string, unknown>) {
    const response = await rawApi<LegacyListResponse<CollaborationLogsResult["items"][number]>>("/api/remote-sync/logs", {
      method: "GET",
      query: params,
    })
    const result = assertSuccess(response, "加载同步日志失败")
    return {
      items: result.items ?? [],
      total: result.total ?? 0,
      limit: result.limit ?? 0,
      offset: result.offset ?? 0,
    } satisfies CollaborationLogsResult
  },

  async getDailyStats(params: { env_id: string; days: number }) {
    const response = await rawApi<LegacyListResponse<CollaborationDailyStat>>("/api/remote-sync/stats/daily", {
      method: "GET",
      query: params,
    })
    return assertSuccess(response, "加载同步统计失败").items ?? []
  },

  async getFlowStats(params: { env_id: string; limit?: number }) {
    const response = await rawApi<LegacyListResponse<CollaborationFlowStat>>("/api/remote-sync/stats/flows", {
      method: "GET",
      query: params,
    })
    return assertSuccess(response, "加载同步流向失败").items ?? []
  },

  async getSiteMetadata(siteId: string) {
    const response = await rawApi<CollaborationSiteMetadataResponse>("/api/remote-sync/sites/" + siteId + "/metadata", {
      method: "GET",
    })
    return assertSuccess(response, "加载站点元数据失败")
  },

  async testEnvMqtt(id: string) {
    return rawApi<CollaborationDiagnosticResponse>("/api/remote-sync/envs/" + id + "/test-mqtt", {
      method: "POST",
    })
  },

  async testEnvHttp(id: string) {
    return rawApi<CollaborationDiagnosticResponse>("/api/remote-sync/envs/" + id + "/test-http", {
      method: "POST",
    })
  },

  async testSiteHttp(siteId: string) {
    return rawApi<CollaborationDiagnosticResponse>("/api/remote-sync/sites/" + siteId + "/test-http", {
      method: "POST",
    })
  },

  async importEnvFromDbOption() {
    const response = await rawApi<CollaborationActionResult>("/api/remote-sync/envs/import-from-dboption", {
      method: "POST",
    })
    return assertSuccess(response, "导入当前配置失败")
  },

  async applyEnv(id: string) {
    const response = await rawApi<CollaborationActionResult>("/api/remote-sync/envs/" + id + "/apply", {
      method: "POST",
    })
    return assertSuccess(response, "应用协同组失败")
  },

  async activateEnv(id: string) {
    const response = await rawApi<ActivateEnvResponse>("/api/remote-sync/envs/" + id + "/activate", {
      method: "POST",
    })
    return assertSuccess(response, "激活协同组失败")
  },

  async stopRuntime() {
    const response = await rawApi<CollaborationActionResult>("/api/remote-sync/runtime/stop", {
      method: "POST",
    })
    return assertSuccess(response, "停止运行时失败")
  },

  async createEnv(payload: CreateCollaborationEnvRequest) {
    const response = await rawApi<CreateResponse>("/api/remote-sync/envs", {
      method: "POST",
      body: payload as unknown as Record<string, unknown>,
    })
    return assertSuccess(response, "创建协同组失败")
  },

  async updateEnv(id: string, payload: UpdateCollaborationEnvRequest) {
    const response = await rawApi<LegacyStatusResponse>("/api/remote-sync/envs/" + id, {
      method: "PUT",
      body: payload as unknown as Record<string, unknown>,
    })
    return assertSuccess(response, "更新协同组失败")
  },

  async createSite(envId: string, payload: CreateCollaborationSiteRequest) {
    const response = await rawApi<CreateResponse>("/api/remote-sync/envs/" + envId + "/sites", {
      method: "POST",
      body: payload as unknown as Record<string, unknown>,
    })
    return assertSuccess(response, "创建协同站点失败")
  },

  async updateSite(siteId: string, payload: UpdateCollaborationSiteRequest) {
    const response = await rawApi<LegacyStatusResponse>("/api/remote-sync/sites/" + siteId, {
      method: "PUT",
      body: payload as unknown as Record<string, unknown>,
    })
    return assertSuccess(response, "更新协同站点失败")
  },

  async deleteSite(siteId: string) {
    const response = await rawApi<LegacyStatusResponse>("/api/remote-sync/sites/" + siteId, {
      method: "DELETE",
    })
    return assertSuccess(response, "删除协同站点失败")
  },

  async deleteEnv(id: string) {
    const response = await rawApi<LegacyStatusResponse>("/api/remote-sync/envs/" + id, {
      method: "DELETE",
    })
    return assertSuccess(response, "删除协同组失败")
  },
}
