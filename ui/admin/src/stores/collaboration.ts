import { computed, ref } from "vue"
import { defineStore } from "pinia"
import { collaborationApi } from "@/api/collaboration"
import type {
  CollaborationControlMessage,
  CollaborationDailyStat,
  CollaborationDiagnosticCheck,
  CollaborationDiagnosticResponse,
  CollaborationDiagnosticsSummary,
  CollaborationEnv,
  CollaborationEnvDiagnostics,
  CollaborationEffectiveStateSummary,
  CollaborationFlowStat,
  CollaborationGroupListItem,
  CollaborationInsightsSummary,
  CollaborationLogFilters,
  CollaborationLogRecord,
  CollaborationOption,
  CollaborationOverviewMetric,
  CollaborationRuntimeConfig,
  CollaborationRuntimeStatus,
  CollaborationSite,
  CollaborationSiteCard,
  CollaborationSiteDiagnostics,
  CollaborationSiteMetadataResponse,
  CollaborationSiteMetadataState,
  CollaborationTone,
  CreateCollaborationEnvRequest,
  CreateCollaborationSiteRequest,
  UpdateCollaborationEnvRequest,
  UpdateCollaborationSiteRequest,
} from "@/types/collaboration"

const DEFAULT_LOG_LIMIT = 20
const DEFAULT_LOG_FILTERS: CollaborationLogFilters = {
  status: "",
  direction: "",
  target_site: "",
  keyword: "",
}

const LOG_STATUS_OPTIONS: CollaborationOption[] = [
  { value: "", label: "全部状态" },
  { value: "completed", label: "成功" },
  { value: "failed", label: "失败" },
  { value: "running", label: "进行中" },
  { value: "pending", label: "等待中" },
]

const LOG_DIRECTION_OPTIONS: CollaborationOption[] = [
  { value: "", label: "全部方向" },
  { value: "push", label: "推送" },
  { value: "pull", label: "拉取" },
  { value: "upload", label: "上传" },
  { value: "download", label: "下载" },
]

function parseNumberList(value: string | null | undefined): number[] {
  if (value == null || value === "") {
    return []
  }

  return value
    .split(/[\s,]+/)
    .map((item) => Number(item.trim()))
    .filter((item) => Number.isFinite(item))
}

function normalizeTextValue(value: string | null | undefined) {
  if (value == null) return null
  const normalized = value.trim()
  return normalized === "" ? null : normalized
}

function normalizeNullableNumber(value: number | null | undefined) {
  return typeof value === "number" && Number.isFinite(value) ? value : null
}

function normalizeNumberArray(values: number[]) {
  return [...new Set(values.filter((value) => Number.isFinite(value)))].sort((left, right) => left - right)
}

function areNumberArraysEqual(left: number[], right: number[]) {
  if (left.length !== right.length) return false
  return left.every((value, index) => value === right[index])
}

function matchesRuntimeConfig(env: CollaborationEnv | null, runtimeConfig: CollaborationRuntimeConfig | null) {
  if (!env || !runtimeConfig) return false

  return normalizeTextValue(env.mqtt_host) === normalizeTextValue(runtimeConfig.mqtt_host)
    && normalizeNullableNumber(env.mqtt_port) === normalizeNullableNumber(runtimeConfig.mqtt_port)
    && normalizeTextValue(env.file_server_host) === normalizeTextValue(runtimeConfig.file_server_host)
    && normalizeTextValue(env.location) === normalizeTextValue(runtimeConfig.location)
    && areNumberArraysEqual(
      normalizeNumberArray(parseNumberList(env.location_dbs)),
      normalizeNumberArray(runtimeConfig.location_dbs),
    )
}

function sumBy<T>(items: T[], getter: (item: T) => number) {
  return items.reduce((total, item) => total + getter(item), 0)
}

function maxBy<T>(items: T[], getter: (item: T) => number) {
  if (items.length === 0) {
    return null
  }

  return items.reduce((best, current) => {
    return getter(current) > getter(best) ? current : best
  })
}

function upsertEnv(envs: CollaborationEnv[], nextEnv: CollaborationEnv) {
  const index = envs.findIndex((item) => item.id === nextEnv.id)
  if (index === -1) {
    envs.unshift(nextEnv)
    return
  }
  envs[index] = nextEnv
}

function buildStatusLabel(isActive: boolean, mqttConnected: boolean | null): { label: string; tone: CollaborationTone } {
  if (isActive && mqttConnected === false) {
    return { label: "MQTT 未连接", tone: "warning" }
  }
  if (isActive) {
    return { label: "已激活", tone: "success" }
  }
  return { label: "未激活", tone: "default" }
}

function metadataSourceLabel(source: string | null) {
  switch (source) {
    case "remote_http":
      return "远端直连"
    case "cache":
      return "缓存回退"
    case "local_path":
      return "本地文件"
    default:
      return "暂无元数据"
  }
}

function availabilityFromMetadata(metadata: CollaborationSiteMetadataState | undefined) {
  if (metadata == null) {
    return { value: "offline" as const, label: "离线" }
  }
  if (metadata.state === "error") {
    return { value: "offline" as const, label: "离线" }
  }
  if (metadata.source === "remote_http" || metadata.source === "local_path") {
    return { value: "online" as const, label: "在线" }
  }
  if (metadata.source === "cache") {
    return { value: "cached" as const, label: "缓存" }
  }
  return { value: "offline" as const, label: "离线" }
}

function normalizeMetadataState(siteId: string, response: CollaborationSiteMetadataResponse): CollaborationSiteMetadataState {
  const latestUpdatedAt = response.metadata.entries
    .map((entry) => entry.updated_at)
    .filter((value) => value !== "")
    .sort()
    .at(-1) ?? null

  return {
    siteId,
    state: "ready",
    source: response.source,
    fetchedAt: response.fetched_at,
    entryCount: response.entry_count,
    totalRecordCount: sumBy(response.metadata.entries, (entry) => entry.record_count ?? 0),
    latestUpdatedAt,
    warningCount: response.warnings.length,
    message: response.warnings[0] ?? null,
  }
}

function emptyDiagnosticCheck(status: CollaborationDiagnosticCheck["status"] = "idle", message = ""): CollaborationDiagnosticCheck {
  return {
    status,
    message,
    checkedAt: null,
    addr: null,
    url: null,
    code: null,
    latencyMs: null,
  }
}

function normalizeDiagnosticCheck(response: CollaborationDiagnosticResponse, fallbackMessage: string): CollaborationDiagnosticCheck {
  return {
    status: response.status === "success" ? "success" : "failed",
    message: response.message || fallbackMessage,
    checkedAt: response.checked_at ?? null,
    addr: response.addr ?? null,
    url: response.url ?? null,
    code: typeof response.code === "number" ? response.code : null,
    latencyMs: typeof response.latency_ms === "number" ? response.latency_ms : null,
  }
}

function diagnosticFromError(error: unknown, fallbackMessage: string): CollaborationDiagnosticCheck {
  return {
    status: "failed",
    message: error instanceof Error ? error.message : fallbackMessage,
    checkedAt: new Date().toISOString(),
    addr: null,
    url: null,
    code: null,
    latencyMs: null,
  }
}

function diagnosticLabel(status: CollaborationDiagnosticCheck["status"]) {
  switch (status) {
    case "success":
      return "正常"
    case "failed":
      return "失败"
    case "running":
      return "测试中"
    default:
      return "未诊断"
  }
}

function diagnosticTone(status: CollaborationDiagnosticCheck["status"]): CollaborationTone {
  switch (status) {
    case "success":
      return "success"
    case "failed":
      return "danger"
    case "running":
      return "warning"
    default:
      return "default"
  }
}

function parseCheckedAt(value: string | null | undefined) {
  if (!value) return 0
  const parsed = Date.parse(value)
  return Number.isFinite(parsed) ? parsed : 0
}

function createEmptyEnvDiagnostics(): CollaborationEnvDiagnostics {
  return {
    mqtt: emptyDiagnosticCheck(),
    http: emptyDiagnosticCheck(),
  }
}

function shouldRefreshLogs(patch: Partial<CollaborationLogFilters>) {
  return Object.keys(patch).some((key) => key !== "keyword")
}

export const useCollaborationStore = defineStore("collaboration", () => {
  const envs = ref<CollaborationEnv[]>([])
  const envSiteCounts = ref<Record<string, number>>({})
  const selectedEnvId = ref<string | null>(null)

  const runtimeStatus = ref<CollaborationRuntimeStatus | null>(null)
  const runtimeConfig = ref<CollaborationRuntimeConfig | null>(null)

  const sites = ref<CollaborationSite[]>([])
  const logs = ref<CollaborationLogRecord[]>([])
  const logsTotal = ref(0)
  const dailyStats7 = ref<CollaborationDailyStat[]>([])
  const dailyStats14 = ref<CollaborationDailyStat[]>([])
  const flowStats = ref<CollaborationFlowStat[]>([])
  const metadataBySiteId = ref<Record<string, CollaborationSiteMetadataState>>({})

  const envDiagnostics = ref<CollaborationEnvDiagnostics>(createEmptyEnvDiagnostics())
  const siteDiagnosticsById = ref<Record<string, CollaborationSiteDiagnostics>>({})
  const diagnosing = ref(false)

  const loading = ref(false)
  const detailLoading = ref(false)
  const logsLoading = ref(false)
  const error = ref("")
  const detailError = ref("")
  const refreshing = ref(false)
  const activating = ref(false)
  const deleting = ref(false)
  const importing = ref(false)
  const applying = ref(false)
  const stopping = ref(false)
  const lastControlMessage = ref<CollaborationControlMessage>({
    status: "idle",
    message: "尚无运行控制操作",
    at: null,
  })

  const logFilters = ref<CollaborationLogFilters>({ ...DEFAULT_LOG_FILTERS })

  const selectedEnv = computed(() => {
    return envs.value.find((env) => env.id === selectedEnvId.value) ?? null
  })

  const diagnosticsSummary = computed<CollaborationDiagnosticsSummary>(() => {
    if (diagnosing.value) {
      return {
        status: "running",
        label: "诊断中",
        detail: "正在检查 MQTT、文件服务与站点 metadata.json",
        checkedAt: null,
      }
    }

    const checks = [
      envDiagnostics.value.mqtt,
      envDiagnostics.value.http,
      ...Object.values(siteDiagnosticsById.value).map((item) => item.check),
    ].filter((item) => item.status !== "idle")

    if (!checks.length) {
      return {
        status: "idle",
        label: "未诊断",
        detail: "点击诊断检查当前协同组与站点可达性",
        checkedAt: null,
      }
    }

    const failedChecks = checks.filter((item) => item.status === "failed")
    const latestCheckedAt = checks
      .map((item) => item.checkedAt)
      .filter((value): value is string => Boolean(value))
      .sort((left, right) => parseCheckedAt(right) - parseCheckedAt(left))[0] ?? null

    if (failedChecks.length > 0) {
      return {
        status: "failed",
        label: `异常 ${failedChecks.length} 项`,
        detail: failedChecks[0]?.message || "存在诊断失败项",
        checkedAt: latestCheckedAt,
      }
    }

    return {
      status: "success",
      label: "检查正常",
      detail: `已检查 ${checks.length} 项可达性`,
      checkedAt: latestCheckedAt,
    }
  })

  const effectiveState = computed<CollaborationEffectiveStateSummary>(() => {
    const activeEnv = runtimeStatus.value?.env_id
      ? envs.value.find((env) => env.id === runtimeStatus.value?.env_id) ?? null
      : null
    const runtimeEnvName = runtimeStatus.value?.active
      ? (activeEnv?.name ?? runtimeStatus.value?.env_id ?? "未知协同组")
      : "未运行"
    const runtimeEnvDetail = runtimeStatus.value?.active
      ? `env_id ${runtimeStatus.value.env_id ?? "-"}`
      : "当前 remote-sync 运行时未启动"
    const configSource = runtimeConfig.value ? "当前 DbOption 配置" : "未读取到运行配置"
    const configSourceDetail = runtimeConfig.value
      ? runtimeConfig.value.source || [
        normalizeTextValue(runtimeConfig.value.mqtt_host) ?? "未配置 MQTT",
        normalizeTextValue(runtimeConfig.value.file_server_host) ?? "未配置文件服务",
      ].join(" · ")
      : "运行态配置快照不可用"

    if (!selectedEnv.value) {
      return {
        label: "未选择协同组",
        tone: "default",
        runtimeEnvName,
        runtimeEnvDetail,
        configSource,
        configSourceDetail,
        relationDetail: "请选择左侧协同组查看当前生效状态",
        lastAction: lastControlMessage.value,
      }
    }

    if (runtimeStatus.value?.active && runtimeStatus.value.env_id === selectedEnv.value.id) {
      return {
        label: "当前运行中",
        tone: "success",
        runtimeEnvName,
        runtimeEnvDetail,
        configSource,
        configSourceDetail,
        relationDetail: "当前选中协同组已启动 watcher + MQTT",
        lastAction: lastControlMessage.value,
      }
    }

    if (runtimeStatus.value?.active && runtimeStatus.value.env_id !== selectedEnv.value.id) {
      return {
        label: "当前运行的是其他协同组",
        tone: "warning",
        runtimeEnvName,
        runtimeEnvDetail,
        configSource,
        configSourceDetail,
        relationDetail: `当前选中协同组为 ${selectedEnv.value.name}，运行中的不是这一组`,
        lastAction: lastControlMessage.value,
      }
    }

    if (matchesRuntimeConfig(selectedEnv.value, runtimeConfig.value)) {
      return {
        label: "已应用未启动",
        tone: "warning",
        runtimeEnvName,
        runtimeEnvDetail,
        configSource,
        configSourceDetail,
        relationDetail: "当前选中协同组已写入 DbOption，但 remote-sync 运行时未启动",
        lastAction: lastControlMessage.value,
      }
    }

    return {
      label: "未应用",
      tone: "default",
      runtimeEnvName,
      runtimeEnvDetail,
      configSource,
      configSourceDetail,
      relationDetail: "当前选中协同组尚未写入当前 DbOption 配置",
      lastAction: lastControlMessage.value,
    }
  })

  const groups = computed<CollaborationGroupListItem[]>(() => {
    return envs.value.map((env) => {
      const isActive = runtimeStatus.value?.env_id === env.id && runtimeStatus.value.active === true
      const status = buildStatusLabel(isActive, runtimeStatus.value?.mqtt_connected ?? null)
      const port = env.mqtt_port ?? runtimeConfig.value?.mqtt_port ?? 1883
      const mqttHost = env.mqtt_host || runtimeConfig.value?.mqtt_host || "-"

      return {
        id: env.id,
        name: env.name,
        location: env.location,
        mqttSummary: mqttHost + ":" + String(port),
        siteCount: envSiteCounts.value[env.id] ?? (selectedEnvId.value === env.id ? sites.value.length : 0),
        updatedAt: env.updated_at,
        isActive,
        statusLabel: status.label,
        statusTone: status.tone,
      }
    })
  })

  const overviewMetrics = computed<CollaborationOverviewMetric[]>(() => {
    const isActive = runtimeStatus.value?.env_id === selectedEnvId.value && runtimeStatus.value.active === true
    const siteCount = sites.value.length
    const mqttConnected = runtimeStatus.value?.mqtt_connected ?? null

    return [
      {
        id: "site-count",
        label: "远端站点数",
        value: String(siteCount),
        detail: siteCount > 0 ? "当前协同组已纳入的远端站点" : "当前协同组暂未配置远端站点",
        tone: siteCount > 0 ? "success" : "default",
      },
      {
        id: "active-status",
        label: "当前激活状态",
        value: isActive ? "已激活" : "未激活",
        detail: isActive ? "运行中的协同组：" + (selectedEnv.value?.name ?? "-") : "同步按钮会激活当前协同组",
        tone: isActive ? "success" : "default",
      },
      {
        id: "mqtt-status",
        label: "MQTT 运行态",
        value: mqttConnected === true ? "已连接" : mqttConnected === false ? "未连接" : "未知",
        detail: runtimeConfig.value?.mqtt_host
          ? runtimeConfig.value.mqtt_host + ":" + String(runtimeConfig.value.mqtt_port ?? 1883)
          : "未读取到运行时配置",
        tone: mqttConnected === true ? "success" : mqttConnected === false ? "warning" : "default",
      },
      {
        id: "diagnostics-status",
        label: "最近诊断结论",
        value: diagnosticsSummary.value.label,
        detail: diagnosticsSummary.value.checkedAt
          ? `${diagnosticsSummary.value.detail} · ${diagnosticsSummary.value.checkedAt}`
          : diagnosticsSummary.value.detail,
        tone: diagnosticTone(diagnosticsSummary.value.status),
      },
    ]
  })

  const siteCards = computed<CollaborationSiteCard[]>(() => {
    const mqttSummary = runtimeConfig.value?.mqtt_host
      ? runtimeConfig.value.mqtt_host + ":" + String(runtimeConfig.value.mqtt_port ?? 1883)
      : "未配置 MQTT"

    return sites.value.map((site) => {
      const metadata = metadataBySiteId.value[site.id]
      const availability = availabilityFromMetadata(metadata)
      const httpHost = site.http_host || null
      const diagnostic = siteDiagnosticsById.value[site.id]?.check ?? emptyDiagnosticCheck()

      return {
        id: site.id,
        name: site.name,
        location: site.location,
        httpHost,
        dbnums: site.dbnums,
        dbnumList: parseNumberList(site.dbnums),
        notes: site.notes,
        roleLabel: selectedEnv.value?.location != null && site.location === selectedEnv.value.location ? "主站" : "协同站点",
        availability: availability.value,
        availabilityLabel: availability.label,
        connectionSummary: [httpHost || "未配置 HTTP Host", mqttSummary].join(" · "),
        metadataSourceLabel: metadataSourceLabel(metadata?.source ?? null),
        fileCount: metadata?.entryCount ?? 0,
        totalRecordCount: metadata?.totalRecordCount ?? 0,
        latestUpdatedAt: metadata?.latestUpdatedAt ?? null,
        warningCount: metadata?.warningCount ?? 0,
        metadataMessage: metadata?.message ?? null,
        diagnosticStatus: diagnostic.status,
        diagnosticStatusLabel: diagnosticLabel(diagnostic.status),
        diagnosticTone: diagnosticTone(diagnostic.status),
        diagnosticCheckedAt: diagnostic.checkedAt,
        diagnosticMessage: diagnostic.message || null,
        diagnosticUrl: diagnostic.url ?? diagnostic.addr ?? null,
        diagnosticCode: diagnostic.code ?? null,
        diagnosticLatencyMs: diagnostic.latencyMs ?? null,
        diagnosticPending: diagnostic.status === "running",
      }
    })
  })

  const insightsSummary = computed<CollaborationInsightsSummary>(() => {
    const total7d = sumBy(dailyStats7.value, (item) => item.total)
    const total14d = sumBy(dailyStats14.value, (item) => item.total)
    const completed = sumBy(dailyStats14.value, (item) => item.completed)
    const failed = sumBy(dailyStats14.value, (item) => item.failed)
    const totalRecords = sumBy(dailyStats14.value, (item) => item.record_count)
    const totalBytes = sumBy(dailyStats14.value, (item) => item.total_bytes)
    const total = completed + failed
    const alertCount = flowStats.value.filter((item) => item.failed > 0).length

    return {
      total7d,
      total14d,
      completed,
      failed,
      successRate: total > 0 ? completed / total : 0,
      totalRecords,
      totalBytes,
      alertCount,
      busiestFlow: maxBy(flowStats.value, (item) => item.total_bytes || item.total) ?? null,
      riskiestFlow: maxBy(flowStats.value, (item) => item.failed) ?? null,
      lastLogAt: logs.value[0]?.created_at ?? null,
      trend14d: [...dailyStats14.value].sort((left, right) => left.day.localeCompare(right.day)),
      topFailedFlows: [...flowStats.value]
        .filter((item) => item.failed > 0)
        .sort((left, right) => right.failed - left.failed || right.total_bytes - left.total_bytes || right.total - left.total)
        .slice(0, 5),
      recentFailures: logs.value.filter((item) => item.status === "failed").slice(0, 5),
    }
  })

  const logStatusOptions = computed(() => LOG_STATUS_OPTIONS)
  const logDirectionOptions = computed(() => LOG_DIRECTION_OPTIONS)
  const logTargetOptions = computed<CollaborationOption[]>(() => {
    const values = new Set<string>()
    for (const site of sites.value) {
      values.add(site.name)
    }
    for (const item of logs.value) {
      if (item.target_site != null && item.target_site !== "") {
        values.add(item.target_site)
      }
    }

    return [
      { value: "", label: "全部目标站点" },
      ...Array.from(values)
        .sort((left, right) => left.localeCompare(right))
        .map((value) => ({ value, label: value })),
    ]
  })

  function clearDiagnostics() {
    envDiagnostics.value = createEmptyEnvDiagnostics()
    siteDiagnosticsById.value = {}
    diagnosing.value = false
  }

  function syncSiteDiagnostics(siteList: CollaborationSite[]) {
    const nextState: Record<string, CollaborationSiteDiagnostics> = {}
    for (const site of siteList) {
      nextState[site.id] = {
        siteId: site.id,
        siteName: site.name,
        check: siteDiagnosticsById.value[site.id]?.check ?? emptyDiagnosticCheck(),
      }
    }
    siteDiagnosticsById.value = nextState
  }

  function clearDetailState(options?: { preserveDiagnostics?: boolean }) {
    sites.value = []
    logs.value = []
    logsTotal.value = 0
    dailyStats7.value = []
    dailyStats14.value = []
    flowStats.value = []
    metadataBySiteId.value = {}
    detailError.value = ""
    if (!options?.preserveDiagnostics) {
      clearDiagnostics()
    }
  }

  function setControlMessage(status: CollaborationControlMessage["status"], message: string) {
    lastControlMessage.value = {
      status,
      message,
      at: new Date().toISOString(),
    }
  }

  async function refreshRuntimeSnapshot() {
    const result = await Promise.all([
      collaborationApi.getRuntimeStatus(),
      collaborationApi.getRuntimeConfig(),
    ])
    runtimeStatus.value = result[0]
    runtimeConfig.value = result[1]
  }

  async function refreshEnvSiteCounts(targetEnvs: CollaborationEnv[] = envs.value) {
    if (targetEnvs.length === 0) {
      envSiteCounts.value = {}
      return
    }

    const results = await Promise.all(
      targetEnvs.map(async (env) => {
        try {
          const envSites = await collaborationApi.listSites(env.id)
          return [env.id, envSites.length] as const
        } catch {
          return [env.id, envSiteCounts.value[env.id] ?? 0] as const
        }
      }),
    )

    envSiteCounts.value = Object.fromEntries(results)
  }

  async function refreshLogs(options?: { silent?: boolean }) {
    if (selectedEnvId.value == null || selectedEnvId.value === "") {
      logs.value = []
      logsTotal.value = 0
      return
    }

    if (options?.silent !== true) {
      logsLoading.value = true
    }

    try {
      const response = await collaborationApi.listLogs({
        env_id: selectedEnvId.value,
        limit: DEFAULT_LOG_LIMIT,
        status: logFilters.value.status || undefined,
        direction: logFilters.value.direction || undefined,
        target_site: logFilters.value.target_site || undefined,
      })

      if (selectedEnvId.value != null && selectedEnvId.value !== "") {
        logs.value = response.items
        logsTotal.value = response.total
      }
    } finally {
      logsLoading.value = false
    }
  }

  async function refreshMetadata(siteList: CollaborationSite[]) {
    const nextState: Record<string, CollaborationSiteMetadataState> = {}
    for (const site of siteList) {
      nextState[site.id] = {
        siteId: site.id,
        state: "loading",
        source: null,
        fetchedAt: null,
        entryCount: 0,
        totalRecordCount: 0,
        latestUpdatedAt: null,
        warningCount: 0,
        message: null,
      }
    }
    metadataBySiteId.value = nextState

    const metadataEntries = await Promise.all(
      siteList.map(async (site) => {
        try {
          const response = await collaborationApi.getSiteMetadata(site.id)
          return [site.id, normalizeMetadataState(site.id, response)] as const
        } catch (err: unknown) {
          return [site.id, {
            siteId: site.id,
            state: "error" as const,
            source: null,
            fetchedAt: null,
            entryCount: 0,
            totalRecordCount: 0,
            latestUpdatedAt: null,
            warningCount: 0,
            message: err instanceof Error ? err.message : "元数据加载失败",
          }] as const
        }
      }),
    )

    metadataBySiteId.value = Object.fromEntries(metadataEntries)
  }

  async function refreshSelectedDetail(options?: { silent?: boolean; resetFilters?: boolean; preserveDiagnostics?: boolean }) {
    const envId = selectedEnvId.value
    if (envId == null || envId === "") {
      clearDetailState({ preserveDiagnostics: options?.preserveDiagnostics })
      return
    }

    if (options?.resetFilters === true) {
      logFilters.value = { ...DEFAULT_LOG_FILTERS }
    }

    if (options?.silent !== true) {
      detailLoading.value = true
    }
    detailError.value = ""

    try {
      const results = await Promise.allSettled([
        collaborationApi.getEnv(envId),
        collaborationApi.listSites(envId),
        collaborationApi.listLogs({
          env_id: envId,
          limit: DEFAULT_LOG_LIMIT,
          status: logFilters.value.status || undefined,
          direction: logFilters.value.direction || undefined,
          target_site: logFilters.value.target_site || undefined,
        }),
        collaborationApi.getDailyStats({ env_id: envId, days: 7 }),
        collaborationApi.getDailyStats({ env_id: envId, days: 14 }),
        collaborationApi.getFlowStats({ env_id: envId, limit: 20 }),
      ])

      if (selectedEnvId.value !== envId) {
        return
      }

      const envResult = results[0]
      const sitesResult = results[1]
      const logsResult = results[2]
      const daily7Result = results[3]
      const daily14Result = results[4]
      const flowResult = results[5]

      if (envResult.status === "fulfilled" && envResult.value != null) {
        upsertEnv(envs.value, envResult.value)
      }

      let nextSites: CollaborationSite[] = []
      let hardFailure = false

      if (sitesResult.status === "fulfilled") {
        nextSites = sitesResult.value
        sites.value = nextSites
        envSiteCounts.value = { ...envSiteCounts.value, [envId]: nextSites.length }
        syncSiteDiagnostics(nextSites)
      } else {
        hardFailure = true
        sites.value = []
        syncSiteDiagnostics([])
      }

      if (logsResult.status === "fulfilled") {
        logs.value = logsResult.value.items
        logsTotal.value = logsResult.value.total
      } else {
        logs.value = []
        logsTotal.value = 0
      }

      dailyStats7.value = daily7Result.status === "fulfilled" ? daily7Result.value : []
      dailyStats14.value = daily14Result.status === "fulfilled" ? daily14Result.value : []
      flowStats.value = flowResult.status === "fulfilled" ? flowResult.value : []

      await refreshRuntimeSnapshot()
      await refreshMetadata(nextSites)

      if (hardFailure) {
        detailError.value = "协同组详情加载不完整，请稍后重试"
      }
    } catch (err: unknown) {
      detailError.value = err instanceof Error ? err.message : "协同组详情加载失败"
      clearDetailState({ preserveDiagnostics: options?.preserveDiagnostics ?? true })
    } finally {
      detailLoading.value = false
      logsLoading.value = false
    }
  }

  async function initialize(preferredEnvId?: string | null) {
    loading.value = true
    error.value = ""

    try {
      const previousSelection = selectedEnvId.value
      const result = await Promise.all([
        collaborationApi.listEnvs(),
        refreshRuntimeSnapshot(),
      ])
      const envList = result[0]

      envs.value = envList
      await refreshEnvSiteCounts(envList)

      const hasPreferred = preferredEnvId != null && preferredEnvId !== "" && envList.some((env) => env.id === preferredEnvId)
      const hasCurrent = selectedEnvId.value != null && selectedEnvId.value !== "" && envList.some((env) => env.id === selectedEnvId.value)
      const nextSelection = hasPreferred
        ? preferredEnvId
        : hasCurrent
          ? selectedEnvId.value
          : envList[0]?.id ?? null

      selectedEnvId.value = nextSelection
      if (previousSelection !== nextSelection) {
        clearDiagnostics()
      }
      if (nextSelection != null && nextSelection !== "") {
        await refreshSelectedDetail({ resetFilters: true, preserveDiagnostics: true })
      } else {
        clearDetailState()
      }
    } catch (err: unknown) {
      error.value = err instanceof Error ? err.message : "协同组加载失败"
      envs.value = []
      selectedEnvId.value = null
      clearDetailState()
    } finally {
      loading.value = false
    }
  }

  async function selectEnv(envId: string | null) {
    if (envId == null || envId === "") {
      selectedEnvId.value = null
      clearDetailState()
      return
    }

    if (selectedEnvId.value === envId) {
      return
    }

    selectedEnvId.value = envId
    clearDiagnostics()
    await refreshSelectedDetail({ resetFilters: true, preserveDiagnostics: true })
  }

  async function refreshAll() {
    refreshing.value = true
    error.value = ""

    try {
      const previousSelection = selectedEnvId.value
      const envList = await collaborationApi.listEnvs()
      envs.value = envList
      await refreshEnvSiteCounts(envList)
      await refreshRuntimeSnapshot()

      if (selectedEnvId.value != null && selectedEnvId.value !== "" && envList.some((env) => env.id === selectedEnvId.value) === false) {
        selectedEnvId.value = envList[0]?.id ?? null
      }

      if (previousSelection !== selectedEnvId.value) {
        clearDiagnostics()
      }

      if (selectedEnvId.value != null && selectedEnvId.value !== "") {
        await refreshSelectedDetail({ silent: true, preserveDiagnostics: true })
      } else {
        clearDetailState()
      }
    } catch (err: unknown) {
      error.value = err instanceof Error ? err.message : "协同组刷新失败"
    } finally {
      refreshing.value = false
    }
  }

  async function runDiagnostics() {
    if (selectedEnvId.value == null || selectedEnvId.value === "") {
      return
    }
    const envId = selectedEnvId.value

    diagnosing.value = true
    envDiagnostics.value = {
      mqtt: { ...(envDiagnostics.value.mqtt ?? emptyDiagnosticCheck()), status: "running", message: "正在检查 MQTT 可达性" },
      http: { ...(envDiagnostics.value.http ?? emptyDiagnosticCheck()), status: "running", message: "等待文件服务检查" },
    }

    syncSiteDiagnostics(sites.value)
    for (const site of sites.value) {
      siteDiagnosticsById.value[site.id] = {
        siteId: site.id,
        siteName: site.name,
        check: { ...(siteDiagnosticsById.value[site.id]?.check ?? emptyDiagnosticCheck()), status: "running", message: "正在检查 metadata.json" },
      }
    }

    try {
      envDiagnostics.value = {
        ...envDiagnostics.value,
        mqtt: normalizeDiagnosticCheck(await collaborationApi.testEnvMqtt(envId), "MQTT 诊断失败"),
        http: { ...(envDiagnostics.value.http ?? emptyDiagnosticCheck()), status: "running", message: "正在检查文件服务可达性" },
      }
    } catch (error) {
      envDiagnostics.value = {
        ...envDiagnostics.value,
        mqtt: diagnosticFromError(error, "MQTT 诊断失败"),
        http: { ...(envDiagnostics.value.http ?? emptyDiagnosticCheck()), status: "running", message: "正在检查文件服务可达性" },
      }
    }

    try {
      envDiagnostics.value = {
        ...envDiagnostics.value,
        http: normalizeDiagnosticCheck(await collaborationApi.testEnvHttp(envId), "文件服务诊断失败"),
      }
    } catch (error) {
      envDiagnostics.value = {
        ...envDiagnostics.value,
        http: diagnosticFromError(error, "文件服务诊断失败"),
      }
    }

    const CONCURRENCY_LIMIT = 5
    const siteQueue = [...sites.value]
    const runBatch = async () => {
      while (siteQueue.length > 0) {
        const batch = siteQueue.splice(0, CONCURRENCY_LIMIT)
        await Promise.all(
          batch.map(async (site) => {
            try {
              const response = await collaborationApi.testSiteHttp(site.id)
              if (selectedEnvId.value !== envId) {
                return
              }
              siteDiagnosticsById.value[site.id] = {
                siteId: site.id,
                siteName: site.name,
                check: normalizeDiagnosticCheck(response, "站点诊断失败"),
              }
            } catch (error) {
              siteDiagnosticsById.value[site.id] = {
                siteId: site.id,
                siteName: site.name,
                check: diagnosticFromError(error, "站点诊断失败"),
              }
            }
          }),
        )
      }
    }
    await runBatch()

    diagnosing.value = false
  }

  async function runSiteDiagnostic(siteId: string) {
    const site = sites.value.find((item) => item.id === siteId)
    if (!site) {
      return
    }

    siteDiagnosticsById.value[siteId] = {
      siteId,
      siteName: site.name,
      check: { ...(siteDiagnosticsById.value[siteId]?.check ?? emptyDiagnosticCheck()), status: "running", message: "正在检查 metadata.json" },
    }

    try {
      const response = await collaborationApi.testSiteHttp(siteId)
      siteDiagnosticsById.value[siteId] = {
        siteId,
        siteName: site.name,
        check: normalizeDiagnosticCheck(response, "站点诊断失败"),
      }
    } catch (error) {
      siteDiagnosticsById.value[siteId] = {
        siteId,
        siteName: site.name,
        check: diagnosticFromError(error, "站点诊断失败"),
      }
    }
  }

  async function importEnvFromDbOption() {
    importing.value = true
    try {
      const response = await collaborationApi.importEnvFromDbOption()
      setControlMessage("success", response.message || "已从当前 DbOption 导入协同组")
      await initialize(response.id ?? null)
      return response.id ?? null
    } catch (err) {
      setControlMessage("failed", err instanceof Error ? err.message : "导入当前配置失败")
      throw err
    } finally {
      importing.value = false
    }
  }

  async function applySelectedEnv() {
    if (selectedEnvId.value == null || selectedEnvId.value === "") {
      return
    }

    applying.value = true
    try {
      const response = await collaborationApi.applyEnv(selectedEnvId.value)
      await refreshRuntimeSnapshot()
      setControlMessage("success", response.message || "已写入当前 DbOption 配置")
    } catch (err) {
      setControlMessage("failed", err instanceof Error ? err.message : "应用协同组失败")
      throw err
    } finally {
      applying.value = false
    }
  }

  async function activateSelectedEnv() {
    if (selectedEnvId.value == null || selectedEnvId.value === "") {
      return
    }

    activating.value = true
    try {
      const response = await collaborationApi.activateEnv(selectedEnvId.value)
      setControlMessage("success", response.message || "已启动当前协同组运行时")
      await refreshAll()
    } catch (err) {
      setControlMessage("failed", err instanceof Error ? err.message : "启动协同组失败")
      throw err
    } finally {
      activating.value = false
    }
  }

  async function stopCurrentRuntime() {
    stopping.value = true
    try {
      const response = await collaborationApi.stopRuntime()
      await refreshRuntimeSnapshot()
      setControlMessage("success", response.message || "已停止当前运行时")
    } catch (err) {
      setControlMessage("failed", err instanceof Error ? err.message : "停止运行时失败")
      throw err
    } finally {
      stopping.value = false
    }
  }

  async function deleteSelectedEnv() {
    if (selectedEnvId.value == null || selectedEnvId.value === "") {
      return
    }

    deleting.value = true
    const deletingId = selectedEnvId.value
    try {
      await collaborationApi.deleteEnv(deletingId)
      envs.value = envs.value.filter((env) => env.id !== deletingId)
      delete envSiteCounts.value[deletingId]
      selectedEnvId.value = envs.value[0]?.id ?? null
      clearDiagnostics()
      await refreshRuntimeSnapshot()
      if (selectedEnvId.value != null && selectedEnvId.value !== "") {
        await refreshSelectedDetail({ resetFilters: true, preserveDiagnostics: true })
      } else {
        clearDetailState()
      }
    } finally {
      deleting.value = false
    }
  }

  async function createEnv(payload: CreateCollaborationEnvRequest) {
    const response = await collaborationApi.createEnv(payload)
    await initialize(response.id ?? null)
    return response.id ?? null
  }

  async function updateEnv(id: string, payload: UpdateCollaborationEnvRequest) {
    await collaborationApi.updateEnv(id, payload)
    await refreshAll()
  }

  async function createSite(payload: CreateCollaborationSiteRequest) {
    if (selectedEnvId.value == null || selectedEnvId.value === "") {
      throw new Error("请先选择协同组")
    }

    const response = await collaborationApi.createSite(selectedEnvId.value, payload)
    await refreshSelectedDetail({ silent: true, preserveDiagnostics: true })
    return response.id ?? null
  }

  async function updateSite(siteId: string, payload: UpdateCollaborationSiteRequest) {
    await collaborationApi.updateSite(siteId, payload)
    await refreshSelectedDetail({ silent: true, preserveDiagnostics: true })
  }

  async function deleteSite(siteId: string) {
    await collaborationApi.deleteSite(siteId)
    await refreshSelectedDetail({ silent: true, preserveDiagnostics: true })
  }

  async function setLogFilters(patch: Partial<CollaborationLogFilters>) {
    logFilters.value = {
      ...logFilters.value,
      ...patch,
    }
    if (shouldRefreshLogs(patch)) {
      await refreshLogs()
    }
  }

  async function pollCurrentSelection() {
    try {
      await refreshRuntimeSnapshot()
      if (selectedEnvId.value != null && selectedEnvId.value !== "") {
        await refreshSelectedDetail({ silent: true, preserveDiagnostics: true })
      }
    } catch {
      // 轮询静默失败，避免打断当前界面
    }
  }

  return {
    envs,
    selectedEnvId,
    runtimeStatus,
    runtimeConfig,
    sites,
    logs,
    logsTotal,
    loading,
    detailLoading,
    logsLoading,
    diagnosticsRunning: diagnosing,
    diagnosing,
    error,
    detailError,
    refreshing,
    activating,
    deleting,
    importing,
    applying,
    stopping,
    logFilters,
    envDiagnostics,
    diagnosticsSummary,
    effectiveState,
    lastControlMessage,
    selectedEnv,
    groups,
    overviewMetrics,
    siteCards,
    insightsSummary,
    logStatusOptions,
    logDirectionOptions,
    logTargetOptions,
    initialize,
    selectEnv,
    refreshAll,
    refreshLogs,
    runDiagnostics,
    runSiteDiagnostic,
    importEnvFromDbOption,
    applySelectedEnv,
    activateSelectedEnv,
    stopCurrentRuntime,
    deleteSelectedEnv,
    createEnv,
    updateEnv,
    createSite,
    updateSite,
    deleteSite,
    setLogFilters,
    pollCurrentSelection,
  }
})
