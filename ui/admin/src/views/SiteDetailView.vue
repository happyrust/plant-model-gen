<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import {
  AlertTriangle,
  CheckCircle2,
  Circle,
  Database,
  FolderArchive,
  Globe,
  HardDrive,
  ListChecks,
  Loader2,
  Monitor,
  ShieldAlert,
  TimerReset,
  XCircle,
} from 'lucide-vue-next'
import { extractErrorMessage } from '@/api/client'
import { sitesApi, type ManagedSiteLogKind } from '@/api/sites'
import { tasksApi } from '@/api/tasks'
import { usePolling } from '@/composables/usePolling'
import { useAdminSitesStream } from '@/composables/useAdminSitesStream'
import { useSitesStore } from '@/stores/sites'
import SiteDetailHeader from '@/components/sites/SiteDetailHeader.vue'
import SiteRuntimeCards from '@/components/sites/SiteRuntimeCards.vue'
import SiteLogSummaryPanel from '@/components/sites/SiteLogSummaryPanel.vue'
import SiteRecentActivityPanel from '@/components/sites/SiteRecentActivityPanel.vue'
import SiteConfigSections from '@/components/sites/SiteConfigSections.vue'
import SiteDrawer from '@/components/sites/SiteDrawer.vue'
import { matchParsePreset, parseDbTypeLabelMap, splitParseDbTypes } from '@/components/sites/parse-db-types'
import { parsePlanClass, siteActionLabelMap } from '@/components/sites/site-status'
import { buildViewerUrl } from '@/lib/viewer'
import type {
  ManagedProjectSite,
  ManagedSiteDeployValidationCheck,
  ManagedSiteDeployValidationReport,
  ManagedSiteLogsResponse,
  ManagedRemoteDeployStatus,
  ManagedRemoteTargetRequest,
  ManagedSitePreflightCheck,
  ManagedSitePreflightReport,
  ManagedSiteProcessResource,
  ManagedSiteRiskLevel,
  ManagedSiteRuntimeStatus,
} from '@/types/site'
import type { TaskInfo, TaskStatus } from '@/types/task'

const route = useRoute()
const router = useRouter()
const sitesStore = useSitesStore()

const site = ref<ManagedProjectSite | null>(null)
const runtime = ref<ManagedSiteRuntimeStatus | null>(null)
const logsData = ref<ManagedSiteLogsResponse | null>(null)
const preflight = ref<ManagedSitePreflightReport | null>(null)
const deployValidation = ref<ManagedSiteDeployValidationReport | null>(null)
const deployTask = ref<TaskInfo | null>(null)
const remoteDeployStatus = ref<ManagedRemoteDeployStatus | null>(null)
const remoteTargetForm = ref<ManagedRemoteTargetRequest>({
  id: 'default',
  name: '默认 Linux 目标',
  host: '123.57.182.243',
  ssh_port: 22,
  ssh_user: 'root',
  password_env: 'REMOTE_PASS',
  remote_root: '/opt/plant3d/sites',
  remote_db_path: '',
  remote_web_port: 3100,
  remote_db_port: 8020,
  public_base_url: '',
  surreal_bin: '/usr/local/bin/surreal',
  remote_web_bin: '/root/web_server',
})
const siteError = ref('')
const runtimeError = ref('')
const logsError = ref('')
const preflightError = ref('')
const deployValidationError = ref('')
const deployTaskError = ref('')
const remoteDeployError = ref('')
const reconcileError = ref('')
const reconcileActions = ref<string[]>([])
const preflightLoading = ref(false)
const deployValidationLoading = ref(false)
const deployTaskLoading = ref(false)
const remotePreflightLoading = ref(false)
const remoteDeployLoading = ref(false)
const reconcileLoading = ref(false)
type DetailTab = 'overview' | 'deploy'

// D6 / Sprint D · 修 G16：tab 状态持久化到 URL `?tab=overview|deploy`
//
// 旧版 activeTab 仅保存在组件 ref，刷新页面回到「运行概览」；新版从 URL query
// 取初值并双向同步，刷新 / 分享链接都能保留 tab 选择。
const initialTab: DetailTab = route.query.tab === 'deploy' ? 'deploy' : 'overview'
const activeTab = ref<DetailTab>(initialTab)
const initialDeployTaskId = typeof route.query.task_id === 'string' ? route.query.task_id : ''
const deployTaskId = ref(initialDeployTaskId)

watch(activeTab, (next) => {
  if (route.query.tab === next) return
  void router.replace({
    path: route.path,
    query: { ...route.query, tab: next },
  })
}, { flush: 'post' })
watch(activeTab, (next) => {
  if (next === 'deploy' && !preflight.value && !preflightLoading.value) {
    void fetchPreflight()
  }
  if (next === 'deploy' && !deployValidation.value && !deployValidationLoading.value) {
    void fetchDeployValidation()
  }
})
watch(() => route.query.task_id, (next) => {
  deployTaskId.value = typeof next === 'string' ? next : ''
  if (deployTaskId.value) {
    void fetchDeployTask()
  } else {
    deployTask.value = null
    deployTaskError.value = ''
  }
})
const activeLogTab = ref<ManagedSiteLogKind>('parse')
const drawerOpen = ref(false)
const downloadPending = ref(false)
const downloadError = ref('')

// D5 / Sprint D · 修 G13：分页尾部日志 + 全量下载
//
// 旧版直接用 logsData.parse_log/db_log/web_log（后端硬编码 LOG_LINES_LIMIT=120 行，
// 无加载更多、无下载入口）；新版按 tab 单独 fetch tailLog，limit 从 200 起步，
// 用户点"加载更多"按 2 倍递增至上限 5000；下载走 Authorization 头 + blob 路径。
const LOG_LIMIT_INITIAL = 200
const LOG_LIMIT_MAX = 5000

interface DetailLogState {
  lines: string[]
  total: number
  limit: number
  truncated: boolean
  loading: boolean
}

const emptyLogState = (): DetailLogState => ({
  lines: [],
  total: 0,
  limit: LOG_LIMIT_INITIAL,
  truncated: false,
  loading: false,
})

const detailLogs = ref<Record<ManagedSiteLogKind, DetailLogState>>({
  parse: emptyLogState(),
  generate: emptyLogState(),
  db: emptyLogState(),
  web: emptyLogState(),
  viewer: emptyLogState(),
})

const siteId = computed(() => String(route.params.id ?? ''))
const resources = computed(() => runtime.value?.resources ?? null)
const actionError = computed(() => sitesStore.getSiteActionError(siteId.value))
const parsePlan = computed(() => runtime.value?.parse_plan ?? site.value?.parse_plan ?? null)
const groupedParseDbTypes = computed(() => splitParseDbTypes(site.value?.parse_db_types ?? []))
const matchedPreset = computed(() => matchParsePreset(
  site.value?.parse_db_types ?? [],
  site.value?.force_rebuild_system_db ?? false,
))
const deployProgressSteps = computed(() => buildDeployProgressSteps())
const deployTaskPercent = computed(() => Math.round(deployTask.value?.progress.percentage ?? 0))
const remoteBlockingCount = computed(() => remoteDeployStatus.value?.checks.filter((check) => check.status === 'blocking').length ?? 0)
const remoteWarningCount = computed(() => remoteDeployStatus.value?.checks.filter((check) => check.status === 'warning').length ?? 0)
const needsReconcile = computed(() => {
  const r = runtime.value
  if (!r) return false
  return (
    (r.status === 'Running' && (!r.web_running || !r.db_running)) ||
    (r.status === 'Starting' && !r.web_running) ||
    !!r.web_port_conflict ||
    !!r.db_port_conflict ||
    !!r.viewer_port_conflict
  )
})

const selectedLogState = computed(() => detailLogs.value[activeLogTab.value])
const selectedLogs = computed(() => selectedLogState.value.lines)

const processCards = computed(() => [
  {
    key: 'db',
    label: 'DB 进程',
    icon: Database,
    process: resources.value?.db_process ?? null,
  },
  {
    key: 'web',
    label: 'Web 进程',
    icon: Globe,
    process: resources.value?.web_process ?? null,
  },
  {
    key: 'viewer',
    label: 'Viewer 进程',
    icon: Monitor,
    process: resources.value?.viewer_process ?? null,
  },
  {
    key: 'parse',
    label: 'Parse 进程',
    icon: TimerReset,
    process: resources.value?.parse_process ?? null,
  },
])

type DeployStepState = 'complete' | 'current' | 'pending' | 'warning' | 'error' | 'skipped'

interface DeployProgressStep {
  key: string
  label: string
  state: DeployStepState
  detail: string
}

const taskStatusConfig: Record<TaskStatus, { class: string; label: string }> = {
  Pending: { class: 'bg-muted text-muted-foreground', label: '等待中' },
  Running: { class: 'bg-blue-100 text-blue-800 dark:bg-blue-900 dark:text-blue-200', label: '运行中' },
  Completed: { class: 'bg-green-100 text-green-800 dark:bg-green-900 dark:text-green-200', label: '已完成' },
  Failed: { class: 'bg-red-100 text-red-800 dark:bg-red-900 dark:text-red-200', label: '失败' },
  Cancelled: { class: 'bg-muted text-muted-foreground line-through', label: '已取消' },
}

function buildDeployProgressSteps(): DeployProgressStep[] {
  const s = site.value
  const r = runtime.value
  const generationEnabled = !!(s?.gen_model || s?.gen_mesh || s?.gen_spatial_tree)
  const currentStage = r?.current_stage ?? ''
  return [
    {
      key: 'preflight',
      label: '部署预检',
      state: !preflight.value
        ? 'pending'
        : preflight.value.blocking_count > 0
          ? 'error'
          : preflight.value.warning_count > 0
            ? 'warning'
            : 'complete',
      detail: preflight.value
        ? `${preflight.value.blocking_count} 个阻断 / ${preflight.value.warning_count} 个警告`
        : '尚未执行预检',
    },
    {
      key: 'parse',
      label: '项目解析',
      state: r?.parse_status === 'Parsed'
        ? 'complete'
        : r?.parse_status === 'Running'
          ? 'current'
          : r?.parse_status === 'Failed'
            ? 'error'
            : 'pending',
      detail: r?.parse_status === 'Parsed' ? '解析已完成' : r?.current_stage_detail || '等待解析',
    },
    {
      key: 'generate',
      label: '模型生成',
      state: !generationEnabled
        ? 'skipped'
        : currentStage === 'generating'
          ? 'current'
          : r?.status === 'Running' || r?.web_running
            ? 'complete'
            : r?.status === 'Failed'
              ? 'error'
              : 'pending',
      detail: generationEnabled ? (r?.current_stage_detail || '等待生成') : '生成配置未启用，部署时跳过',
    },
    {
      key: 'db',
      label: '启动数据库',
      state: r?.db_running ? 'complete' : r?.status === 'Starting' ? 'current' : 'pending',
      detail: r?.db_running ? `DB 端口 ${r.db_port ?? s?.db_port ?? '-'}` : '等待 DB 进程启动',
    },
    {
      key: 'web',
      label: '启动 Web 服务',
      state: r?.web_running ? 'complete' : r?.status === 'Starting' && r?.db_running ? 'current' : 'pending',
      detail: r?.web_running ? `Web 端口 ${r.web_port ?? s?.web_port ?? '-'}` : '等待 Web 服务 /api/status',
    },
    {
      key: 'viewer',
      label: '启动 Viewer',
      state: r?.viewer_running || r?.viewer_url
        ? 'complete'
        : r?.status === 'Running'
          ? 'warning'
          : r?.web_running
            ? 'current'
            : 'pending',
      detail: r?.viewer_url || '等待 plant3d-web Viewer',
    },
  ]
}

function deployStepIcon(step: DeployProgressStep) {
  if (step.state === 'complete') return CheckCircle2
  if (step.state === 'error') return XCircle
  if (step.state === 'warning') return AlertTriangle
  if (step.state === 'current') return Loader2
  return Circle
}

function deployStepClass(step: DeployProgressStep) {
  if (step.state === 'complete') return 'border-emerald-500/40 bg-emerald-500/5 text-emerald-700 dark:text-emerald-300'
  if (step.state === 'error') return 'border-destructive/50 bg-destructive/5 text-destructive'
  if (step.state === 'warning') return 'border-amber-500/40 bg-amber-500/5 text-amber-700 dark:text-amber-300'
  if (step.state === 'current') return 'border-blue-500/40 bg-blue-500/5 text-blue-700 dark:text-blue-300'
  return 'border-border bg-background text-muted-foreground'
}

function preflightCheckClass(check: ManagedSitePreflightCheck) {
  if (check.status === 'blocking') return 'border-destructive/50 bg-destructive/5'
  if (check.status === 'warning') return 'border-amber-500/40 bg-amber-500/5'
  return 'border-emerald-500/30 bg-emerald-500/5'
}

function preflightStatusLabel(status: ManagedSitePreflightCheck['status']) {
  if (status === 'blocking') return '阻断'
  if (status === 'warning') return '警告'
  return '通过'
}

function deployValidationCheckClass(check: ManagedSiteDeployValidationCheck) {
  const status = check.status?.toLowerCase()
  if (status === 'blocking') return 'border-destructive/50 bg-destructive/5'
  if (status === 'warning') return 'border-amber-500/40 bg-amber-500/5'
  return 'border-emerald-500/30 bg-emerald-500/5'
}

function deployValidationStatusLabel(status: string) {
  const normalized = status?.toLowerCase()
  if (normalized === 'blocking') return '阻断'
  if (normalized === 'warning') return '警告'
  if (normalized === 'pass') return '通过'
  return status || '未知'
}

const riskTone = computed(() => toneForRisk(runtime.value?.risk_level ?? 'normal'))
const parseHealthTone = computed(() => {
  const status = runtime.value?.parse_health.status ?? 'unknown'
  if (status === 'critical') return 'text-red-700 dark:text-red-300'
  if (status === 'warning') return 'text-amber-700 dark:text-amber-300'
  if (status === 'normal') return 'text-emerald-700 dark:text-emerald-300'
  return 'text-muted-foreground'
})

async function fetchPreflight() {
  if (!siteId.value) return
  preflightLoading.value = true
  try {
    preflight.value = await sitesApi.preflight(siteId.value)
    preflightError.value = ''
  } catch (err: unknown) {
    preflightError.value = extractErrorMessage(err)
  } finally {
    preflightLoading.value = false
  }
}

async function fetchDeployValidation() {
  if (!siteId.value) return
  deployValidationLoading.value = true
  try {
    deployValidation.value = await sitesApi.deployValidation(siteId.value)
    deployValidationError.value = ''
  } catch (err: unknown) {
    deployValidationError.value = extractErrorMessage(err)
  } finally {
    deployValidationLoading.value = false
  }
}

async function refreshDeployValidation() {
  if (!siteId.value) return
  deployValidationLoading.value = true
  try {
    deployValidation.value = await sitesApi.refreshDeployValidation(siteId.value)
    deployValidationError.value = ''
  } catch (err: unknown) {
    deployValidationError.value = extractErrorMessage(err)
  } finally {
    deployValidationLoading.value = false
  }
}

async function fetchDeployTask() {
  if (!deployTaskId.value) return
  deployTaskLoading.value = true
  try {
    deployTask.value = await tasksApi.get(deployTaskId.value)
    deployTaskError.value = ''
  } catch (err: unknown) {
    deployTaskError.value = extractErrorMessage(err)
  } finally {
    deployTaskLoading.value = false
  }
}


async function fetchRemoteDeployStatus() {
  if (!siteId.value) return
  try {
    remoteDeployStatus.value = await sitesApi.remoteDeployStatus(siteId.value)
    remoteDeployError.value = ''
  } catch (err: unknown) {
    remoteDeployError.value = extractErrorMessage(err)
  }
}

async function handleRemotePreflight() {
  if (!siteId.value) return
  remotePreflightLoading.value = true
  try {
    remoteDeployStatus.value = await sitesApi.remotePreflight(siteId.value, { target: remoteTargetForm.value })
    remoteDeployError.value = ''
  } catch (err: unknown) {
    remoteDeployError.value = extractErrorMessage(err)
  } finally {
    remotePreflightLoading.value = false
  }
}

async function handleRemoteDeploy() {
  if (!siteId.value) return
  remoteDeployLoading.value = true
  try {
    remoteDeployStatus.value = await sitesApi.remotePreflight(siteId.value, { target: remoteTargetForm.value })
    if (remoteDeployStatus.value.status === 'blocked') return
    const submitted = await sitesApi.remoteDeploy(siteId.value, { target: remoteTargetForm.value })
    if (submitted.data?.task_id) {
      await setDeployTaskId(submitted.data.task_id)
      await fetchDeployTask()
    }
    await fetchRemoteDeployStatus()
    remoteDeployError.value = ''
  } catch (err: unknown) {
    remoteDeployError.value = extractErrorMessage(err)
  } finally {
    remoteDeployLoading.value = false
  }
}

async function handleReconcile(cleanupOrphans = false) {
  if (!siteId.value) return
  reconcileLoading.value = true
  reconcileError.value = ''
  try {
    const result = await sitesApi.reconcile(siteId.value, { cleanup_orphans: cleanupOrphans })
    runtime.value = result.runtime
    reconcileActions.value = result.actions
    await fetchAll()
  } catch (err: unknown) {
    reconcileError.value = extractErrorMessage(err)
  } finally {
    reconcileLoading.value = false
  }
}

async function setDeployTaskId(taskId: string) {
  deployTaskId.value = taskId
  await router.replace({
    path: route.path,
    query: { ...route.query, tab: 'deploy', task_id: taskId },
  })
}

async function fetchAll() {
  const id = siteId.value
  try {
    site.value = await sitesApi.get(id)
    siteError.value = ''
  } catch (err: unknown) {
    siteError.value = extractErrorMessage(err)
  }

  try {
    runtime.value = await sitesApi.runtime(id)
    runtimeError.value = ''
  } catch (err: unknown) {
    runtimeError.value = extractErrorMessage(err)
  }

  try {
    logsData.value = await sitesApi.logs(id)
    logsError.value = ''
  } catch (err: unknown) {
    logsError.value = extractErrorMessage(err)
  }

  // 跟随当前 tab 刷新一次详情日志（保留用户已"加载更多"的 limit）
  await fetchKindLog(activeLogTab.value)

  if (deployTaskId.value) {
    await fetchDeployTask()
  }

  if (activeTab.value === 'deploy') {
    await fetchDeployValidation()
  }
}

async function fetchKindLog(kind: ManagedSiteLogKind, overrideLimit?: number) {
  const cur = detailLogs.value[kind]
  const limit = Math.min(overrideLimit ?? cur.limit ?? LOG_LIMIT_INITIAL, LOG_LIMIT_MAX)
  cur.loading = true
  try {
    const r = await sitesApi.tailLog(siteId.value, kind, limit)
    detailLogs.value[kind] = {
      lines: r.lines,
      total: r.total_lines,
      limit: r.limit,
      truncated: r.truncated,
      loading: false,
    }
    logsError.value = ''
  } catch (err: unknown) {
    detailLogs.value[kind].loading = false
    logsError.value = extractErrorMessage(err)
  }
}

function onLogTabChange(tab: ManagedSiteLogKind) {
  activeLogTab.value = tab
  if (detailLogs.value[tab].lines.length === 0 && !detailLogs.value[tab].loading) {
    void fetchKindLog(tab)
  }
}

async function loadMoreLog() {
  const kind = activeLogTab.value
  const cur = detailLogs.value[kind]
  const next = Math.min(cur.limit * 2, LOG_LIMIT_MAX)
  if (next === cur.limit) return
  await fetchKindLog(kind, next)
}

async function downloadLog() {
  const kind = activeLogTab.value
  downloadPending.value = true
  downloadError.value = ''
  try {
    const token = localStorage.getItem('admin_token')
    const url = sitesApi.logDownloadUrl(siteId.value, kind)
    const resp = await fetch(url, {
      headers: token ? { Authorization: `Bearer ${token}` } : {},
    })
    if (!resp.ok) {
      throw new Error(`下载失败 (HTTP ${resp.status})`)
    }
    const blob = await resp.blob()
    const cd = resp.headers.get('content-disposition') ?? ''
    const m = /filename="([^"]+)"/.exec(cd)
    const filename = m?.[1] ?? `${siteId.value}-${kind}.log`
    const objUrl = URL.createObjectURL(blob)
    const a = document.createElement('a')
    a.href = objUrl
    a.download = filename
    document.body.appendChild(a)
    a.click()
    document.body.removeChild(a)
    setTimeout(() => URL.revokeObjectURL(objUrl), 0)
  } catch (err: unknown) {
    downloadError.value = err instanceof Error ? err.message : '下载失败'
  } finally {
    downloadPending.value = false
  }
}

function formatBytes(value?: number | null) {
  if (value === null || value === undefined || value <= 0) return '0 B'
  const units = ['B', 'KB', 'MB', 'GB', 'TB']
  let size = value
  let unitIndex = 0
  while (size >= 1024 && unitIndex < units.length - 1) {
    size /= 1024
    unitIndex += 1
  }
  const formatted = size >= 10 || unitIndex === 0 ? size.toFixed(0) : size.toFixed(1)
  return formatted + ' ' + units[unitIndex]
}

function formatDateTime(value?: string | null) {
  if (value === null || value === undefined || value === '') return '暂无解析记录'
  const date = new Date(value)
  if (Number.isNaN(date.getTime())) return '暂无解析记录'
  return date.toLocaleString('zh-CN', {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
  })
}

function formatDuration(ms?: number | null) {
  if (ms === null || ms === undefined) return '暂无解析记录'
  if (ms < 1000) return String(ms) + ' ms'
  const seconds = Math.floor(ms / 1000)
  if (seconds < 60) return String(seconds) + ' 秒'
  const minutes = Math.floor(seconds / 60)
  return String(minutes) + ' 分 ' + String(seconds % 60) + ' 秒'
}

function formatCpuUsage(process?: ManagedSiteProcessResource | null) {
  if (process?.running !== true) return '未运行'
  if (process.cpu_usage === null || process.cpu_usage === undefined) return '采样中'
  const digits = process.cpu_usage >= 10 ? 0 : 1
  return process.cpu_usage.toFixed(digits) + '%'
}

function formatMemoryUsage(process?: ManagedSiteProcessResource | null) {
  if (process?.running !== true) return '未运行'
  if (process.memory_bytes === null || process.memory_bytes === undefined) return '采样中'
  return formatBytes(process.memory_bytes)
}

function processStatusLabel(process?: ManagedSiteProcessResource | null) {
  if (process?.running !== true) return '未运行'
  if (process.cpu_usage === null || process.cpu_usage === undefined) return '采样中'
  return '运行中'
}

function processStatusClass(process?: ManagedSiteProcessResource | null) {
  if (process?.running !== true) return 'text-muted-foreground'
  if (process.cpu_usage === null || process.cpu_usage === undefined) return 'text-amber-600'
  return 'text-green-600'
}

function toneForRisk(level: ManagedSiteRiskLevel) {
  if (level === 'critical') {
    return {
      badge: 'bg-red-100 text-red-800 dark:bg-red-900 dark:text-red-200',
      card: 'border-red-500/40 bg-red-500/5',
      text: 'text-red-700 dark:text-red-300',
      label: '严重',
    }
  }
  if (level === 'warning') {
    return {
      badge: 'bg-amber-100 text-amber-800 dark:bg-amber-900 dark:text-amber-200',
      card: 'border-amber-500/40 bg-amber-500/5',
      text: 'text-amber-700 dark:text-amber-300',
      label: '警告',
    }
  }
  return {
    badge: 'bg-green-100 text-green-800 dark:bg-green-900 dark:text-green-200',
    card: 'border-emerald-500/40 bg-emerald-500/5',
    text: 'text-emerald-700 dark:text-emerald-300',
    label: '正常',
  }
}

function hasWarning(reason: string) {
  return runtime.value?.warnings.includes(reason) === true
}

function warningTone(reason: string) {
  if (hasWarning(reason) === false) return ''
  if (runtime.value?.risk_level === 'critical') {
    return 'text-red-700 dark:text-red-300'
  }
  return 'text-amber-700 dark:text-amber-300'
}

function processValueTone(label: string, kind: 'cpu' | 'memory') {
  const reason = label + ' 进程' + (kind === 'cpu' ? ' CPU 占用过高' : '内存占用过高')
  return warningTone(reason)
}

function logTabLabel(tab: ManagedSiteLogKind) {
  switch (tab) {
    case 'parse':
      return '解析日志'
    case 'generate':
      return '生成日志'
    case 'db':
      return 'DB 日志'
    case 'web':
      return 'Web 日志'
    case 'viewer':
      return 'Viewer 日志'
  }
}

function viewerUrl() {
  const s = site.value
  if (!s) return null
  return buildViewerUrl(s)
}

function openViewer() {
  const url = viewerUrl()
  if (url) window.open(url, '_blank')
}

function openEditDrawer() {
  if (!site.value) return
  drawerOpen.value = true
}

function handleDrawerSaved() {
  drawerOpen.value = false
  void fetchAll()
}

async function handleStart() {
  try {
    await sitesStore.startSite(siteId.value)
    await fetchAll()
  } catch {
    // 错误已写入 store，页面横幅会显示
  }
}

async function handleStop() {
  try {
    await sitesStore.stopSite(siteId.value)
    await fetchAll()
  } catch {
    // 错误已写入 store，页面横幅会显示
  }
}

async function handleRestart() {
  try {
    await sitesStore.restartSite(siteId.value)
    await fetchAll()
  } catch {
    // 错误已写入 store，页面横幅会显示
  }
}

async function handleParse() {
  try {
    await sitesStore.parseSite(siteId.value)
    await fetchAll()
  } catch {
    // 错误已写入 store，页面横幅会显示
  }
}

async function handleGenerate() {
  try {
    await sitesStore.generateSite(siteId.value)
    activeLogTab.value = 'generate'
    await fetchAll()
  } catch {
    // 错误已写入 store，页面横幅会显示
  }
}

async function handleDeploy() {
  try {
    activeTab.value = 'deploy'
    await fetchPreflight()
    if (preflight.value && !preflight.value.ready) return
    const submitted = await sitesStore.deploySite(siteId.value)
    if (submitted?.task_id) {
      await setDeployTaskId(submitted.task_id)
      await fetchDeployTask()
    }
    activeLogTab.value = 'generate'
    await fetchAll()
    await fetchPreflight()
    await fetchDeployValidation()
    await fetchRemoteDeployStatus()
  } catch {
    // 错误已写入 store，页面横幅会显示
  }
}

function copyText(text: string) {
  navigator.clipboard.writeText(text)
}

// D1 / Sprint D · 修 G7：详情页接入 SSE 实时化
//
// 状态字段（status / parse_status / last_error / project_name）由 SSE
// AdminSiteSnapshot 即时推送 patch，避免 10s polling 才能看到状态翻转。
// 资源指标（runtime.resources）+ 日志（logs）仍走 polling，等 D1 后续
// Phase 加 AdminSiteResource 事件后再剥离。
//
// 仅响应当前路由 site_id 的事件；其他 site 的 snapshot 直接 ignore。
useAdminSitesStream({
  callbacks: {
    onSnapshot: (payload) => {
      if (payload.site_id !== siteId.value) return
      if (!site.value) return
      site.value = {
        ...site.value,
        project_name: payload.project_name ?? site.value.project_name,
        status: payload.status as ManagedProjectSite['status'],
        parse_status: payload.parse_status as ManagedProjectSite['parse_status'],
        last_error: payload.last_error ?? null,
      }
      // runtime（资源 + pid）仍由 10s polling 刷新；SSE 仅负责状态字段 patch
    },
    onDeleted: (payload) => {
      if (payload.site_id !== siteId.value) return
      // 当前详情页对应的 site 被删除（极少见，通常用户在另一个 tab 删除），
      // 跳回列表页。
      void router.replace({ path: '/sites' })
    },
    // 重连成功时由 SitesView 的 onConnect 触发 fetchSites，
    // 详情页只关心当前 site，重连后 polling 自然会拉到最新值。
  },
})

const { start: startPolling } = usePolling(fetchAll, 10000)
const { start: startDeployTaskPolling } = usePolling(async () => {
  if (!deployTaskId.value) return
  await fetchDeployTask()
  if (deployTask.value?.status === 'Completed' || deployTask.value?.status === 'Failed' || deployTask.value?.status === 'Cancelled') {
    await fetchAll()
    await fetchDeployValidation()
    await fetchRemoteDeployStatus()
  }
}, 3000)

onMounted(async () => {
  await fetchAll()
  if (activeTab.value === 'deploy') {
    await fetchPreflight()
    await fetchDeployValidation()
    await fetchRemoteDeployStatus()
  }
  startPolling()
  startDeployTaskPolling()
})
</script>

<template>
  <div class="space-y-6">
    <SiteDetailHeader
      :site="site"
      :viewer-url="viewerUrl()"
      @back="router.push({ path: '/sites' })"
      @start="handleStart"
      @stop="handleStop"
      @restart="handleRestart"
      @parse="handleParse"
      @generate="handleGenerate"
      @deploy="handleDeploy"
      @refresh="fetchAll"
      @open-viewer="openViewer()"
      @edit="openEditDrawer"
    />

    <div
      v-if="siteError"
      class="rounded-lg border border-destructive/50 bg-destructive/5 px-4 py-3 text-sm text-destructive"
    >
      站点信息加载失败：{{ siteError }}
    </div>

    <div
      v-if="actionError"
      class="rounded-lg border border-destructive/50 bg-destructive/5 px-4 py-3 flex items-start justify-between gap-3"
    >
      <div class="flex items-start gap-2 text-sm text-destructive">
        <AlertTriangle class="h-4 w-4 mt-0.5 shrink-0" />
        <span>
          <strong>{{ siteActionLabelMap[actionError.action] }}失败：</strong>
          {{ actionError.message }}
        </span>
      </div>
      <button
        class="inline-flex h-7 items-center gap-1 rounded-md border border-destructive/30 px-2.5 text-xs font-medium text-destructive hover:bg-destructive/10 transition-colors"
        @click="sitesStore.clearSiteActionError(siteId)"
      >
        关闭
      </button>
    </div>

    <div class="flex gap-2 border-b border-border">
      <button
        class="px-4 py-2 text-sm font-medium transition-colors border-b-2"
        :class="activeTab === 'overview' ? 'border-primary text-foreground' : 'border-transparent text-muted-foreground hover:text-foreground'"
        @click="activeTab = 'overview'"
      >运行概览</button>
      <button
        class="px-4 py-2 text-sm font-medium transition-colors border-b-2"
        :class="activeTab === 'deploy' ? 'border-primary text-foreground' : 'border-transparent text-muted-foreground hover:text-foreground'"
        @click="activeTab = 'deploy'"
      >部署进度</button>
    </div>

    <div v-if="activeTab === 'overview'" class="space-y-4">
      <div
        v-if="runtimeError"
        class="rounded-lg border border-amber-500/40 bg-amber-500/5 px-4 py-3 text-sm text-amber-700 dark:text-amber-300"
      >
        运行状态加载失败：{{ runtimeError }}
      </div>

      <SiteRuntimeCards :site="site" :runtime="runtime" />

      <div v-if="parsePlan" class="rounded-lg border border-border bg-card p-5">
        <div class="mb-4 flex items-center gap-2">
          <TimerReset class="h-4 w-4 text-muted-foreground" />
          <h3 class="text-base font-medium">解析计划</h3>
        </div>
        <div class="space-y-4 text-sm">
          <div class="flex flex-wrap items-center gap-3">
            <span
              class="inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium"
              :class="parsePlanClass(parsePlan)"
            >
              {{ parsePlan.label }}
            </span>
            <span class="text-muted-foreground">
              {{
                parsePlan.mode === 'RebuildSystem'
                  ? '会重新解析系统数据'
                  : parsePlan.includes_system_db_files
                    ? '会补齐系统数据'
                    : '只重解析目标数据'
              }}
            </span>
          </div>
          <div class="text-muted-foreground">{{ parsePlan.detail }}</div>
          <div class="rounded-lg border border-border/60 bg-background p-4">
            <div class="text-xs text-muted-foreground">模型数据</div>
            <div v-if="groupedParseDbTypes.model.length" class="mt-2 flex flex-wrap gap-2">
              <span
                v-for="type in groupedParseDbTypes.model"
                :key="type"
                class="inline-flex items-center rounded-full border border-border px-2 py-0.5 text-xs"
              >
                {{ parseDbTypeLabelMap[type] || type }}
              </span>
            </div>
            <div v-else class="mt-2 text-sm text-muted-foreground">未单独限制</div>
          </div>
          <div class="rounded-lg border border-border/60 bg-background p-4">
            <div class="text-xs text-muted-foreground">系统数据</div>
            <div v-if="groupedParseDbTypes.system.length" class="mt-2 flex flex-wrap gap-2">
              <span
                v-for="type in groupedParseDbTypes.system"
                :key="type"
                class="inline-flex items-center rounded-full border border-border px-2 py-0.5 text-xs"
              >
                {{ parseDbTypeLabelMap[type] || type }}
              </span>
            </div>
            <div v-else class="mt-2 text-sm text-muted-foreground">未单独限制</div>
          </div>
          <div v-if="site" class="rounded-lg border border-border/60 bg-background p-4">
            <div class="text-xs text-muted-foreground">系统库策略</div>
            <div class="mt-2 text-sm">
              {{ site.force_rebuild_system_db ? '强制重建 SYST' : '优先复用已解析 SYST' }}
            </div>
          </div>
          <div class="rounded-lg border border-border/60 bg-background p-4">
            <div class="text-xs text-muted-foreground">常用预设</div>
            <div class="mt-2 text-sm">
              {{ matchedPreset?.label || '自定义组合' }}
            </div>
          </div>
          <div class="rounded-lg border border-border/60 bg-background p-4">
            <div class="text-xs text-muted-foreground">当前解析文件</div>
            <div v-if="parsePlan.included_db_files.length" class="mt-2 flex flex-wrap gap-2">
              <span
                v-for="file in parsePlan.included_db_files"
                :key="file"
                class="inline-flex items-center rounded-full border border-border px-2 py-0.5 text-xs"
              >
                {{ file }}
              </span>
            </div>
            <div v-else class="mt-2 text-sm text-muted-foreground">按项目配置做全量解析</div>
          </div>
        </div>
      </div>

      <SiteRecentActivityPanel :runtime="runtime" />

      <div class="rounded-lg border p-5" :class="riskTone.card">
        <div class="mb-4 flex items-center gap-2">
          <ShieldAlert class="h-4 w-4" :class="riskTone.text" />
          <h3 class="text-base font-medium">风险摘要</h3>
        </div>
        <div class="space-y-4 text-sm">
          <div class="flex flex-wrap items-center gap-3">
            <span class="inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium" :class="riskTone.badge">
              {{ riskTone.label }}
            </span>
            <span class="text-muted-foreground">{{ runtime?.warnings.length ? '当前存在明确风险项' : '当前没有明显风险项' }}</span>
          </div>
          <div>
            <div class="text-xs text-muted-foreground">风险原因</div>
            <ul v-if="runtime?.warnings.length" class="mt-2 space-y-1">
              <li v-for="warning in runtime?.warnings" :key="warning" class="flex items-start gap-2">
                <AlertTriangle class="mt-0.5 h-4 w-4" :class="riskTone.text" />
                <span>{{ warning }}</span>
              </li>
            </ul>
            <div v-else class="mt-2 text-muted-foreground">当前没有需要优先处理的风险。</div>
          </div>
          <div class="rounded-lg border border-border/60 bg-background p-4">
            <div class="text-xs text-muted-foreground">解析健康</div>
            <div class="mt-1 text-sm font-medium" :class="parseHealthTone">{{ runtime?.parse_health.label ?? '暂无解析记录' }}</div>
            <div class="mt-1 text-sm text-muted-foreground">{{ runtime?.parse_health.detail ?? '当前没有额外说明。' }}</div>
          </div>
        </div>
      </div>

      <div class="rounded-lg border border-border bg-card p-5">
        <div class="mb-4 flex items-center gap-2">
          <HardDrive class="h-4 w-4 text-muted-foreground" />
          <h3 class="text-base font-medium">进程资源</h3>
        </div>
        <div class="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
          <div v-for="card in processCards" :key="card.key" class="rounded-lg border border-border/60 bg-background p-4">
            <div class="flex items-center justify-between">
              <div class="flex items-center gap-2 text-sm text-muted-foreground">
                <component :is="card.icon" class="h-4 w-4" />
                <span>{{ card.label }}</span>
              </div>
              <span class="text-sm font-medium" :class="processStatusClass(card.process)">
                {{ processStatusLabel(card.process) }}
              </span>
            </div>
            <div class="mt-4 grid gap-3 text-sm">
              <div class="flex items-center justify-between gap-4">
                <span class="text-muted-foreground">PID</span>
                <span>{{ card.process?.pid ?? '-' }}</span>
              </div>
              <div class="flex items-center justify-between gap-4">
                <span class="text-muted-foreground">CPU</span>
                <span :class="processValueTone(card.label, 'cpu')">{{ formatCpuUsage(card.process) }}</span>
              </div>
              <div class="flex items-center justify-between gap-4">
                <span class="text-muted-foreground">内存</span>
                <span :class="processValueTone(card.label, 'memory')">{{ formatMemoryUsage(card.process) }}</span>
              </div>
            </div>
          </div>
        </div>
      </div>

      <div class="rounded-lg border border-border bg-card p-5">
        <div class="mb-4 flex items-center gap-2">
          <FolderArchive class="h-4 w-4 text-muted-foreground" />
          <h3 class="text-base font-medium">目录与解析</h3>
        </div>
        <div class="grid gap-4 lg:grid-cols-2">
          <div class="rounded-lg border border-border/60 bg-background p-4">
            <div class="grid gap-3 text-sm">
              <div class="flex items-center justify-between gap-4">
                <span class="text-muted-foreground">运行目录大小</span>
                <span :class="warningTone('运行目录缺失')">{{ formatBytes(resources?.runtime_dir_size_bytes) }}</span>
              </div>
              <div v-if="resources?.runtime_dir_missing" class="text-xs text-amber-700 dark:text-amber-300">运行目录不存在</div>
              <div class="flex items-center justify-between gap-4">
                <span class="text-muted-foreground">数据目录大小</span>
                <span :class="warningTone('数据目录缺失')">{{ formatBytes(resources?.data_dir_size_bytes) }}</span>
              </div>
              <div v-if="resources?.data_dir_missing" class="text-xs text-amber-700 dark:text-amber-300">数据目录不存在</div>
            </div>
          </div>
          <div class="rounded-lg border border-border/60 bg-background p-4">
            <div class="grid gap-3 text-sm">
              <div class="flex items-center justify-between gap-4">
                <span class="text-muted-foreground">最近解析开始</span>
                <span class="text-right">{{ formatDateTime(resources?.last_parse_started_at) }}</span>
              </div>
              <div class="flex items-center justify-between gap-4">
                <span class="text-muted-foreground">最近解析结束</span>
                <span class="text-right">{{ formatDateTime(resources?.last_parse_finished_at) }}</span>
              </div>
              <div class="flex items-center justify-between gap-4">
                <span class="text-muted-foreground">最近解析耗时</span>
                <span :class="parseHealthTone">{{ formatDuration(resources?.last_parse_duration_ms) }}</span>
              </div>
            </div>
          </div>
        </div>
      </div>

      <div v-if="runtime?.last_error" class="rounded-lg border border-destructive/50 bg-destructive/5 p-4">
        <div class="text-sm font-medium text-destructive">最近错误</div>
        <div class="mt-1 text-sm text-destructive/80">{{ runtime.last_error }}</div>
      </div>

      <div v-if="runtime?.db_port_conflict || runtime?.web_port_conflict || runtime?.viewer_port_conflict" class="rounded-lg border border-amber-500/50 bg-amber-500/5 p-4">
        <div class="text-sm font-medium text-amber-700 dark:text-amber-300 mb-1">端口冲突</div>
        <div v-if="runtime?.web_port_conflict" class="text-sm text-amber-600 dark:text-amber-400">
          Web 端口 {{ runtime.web_port }} 被外部进程占用 (PIDs: {{ runtime.web_conflict_pids?.join(', ') }})
        </div>
        <div v-if="runtime?.viewer_port_conflict" class="text-sm text-amber-600 dark:text-amber-400">
          Viewer 端口 {{ runtime.viewer_port ?? '-' }} 被外部进程占用 (PIDs: {{ runtime.viewer_conflict_pids?.join(', ') }})
        </div>
        <div v-if="runtime?.db_port_conflict" class="text-sm text-amber-600 dark:text-amber-400">
          DB 端口 {{ runtime.db_port }} 被外部进程占用 (PIDs: {{ runtime.db_conflict_pids?.join(', ') }})
        </div>
      </div>

      <div class="rounded-lg border border-border bg-card p-5">
        <div class="flex flex-wrap items-start justify-between gap-3">
          <div>
            <h3 class="text-base font-medium">运行态对账</h3>
            <p class="mt-1 text-sm text-muted-foreground">
              {{ needsReconcile ? '当前状态与进程/端口信号不完全一致，建议先对账。' : '当前没有发现明显半启动或端口残留。' }}
            </p>
          </div>
          <div class="flex flex-wrap items-center gap-2">
            <button
              :disabled="reconcileLoading"
              @click="handleReconcile(false)"
              class="inline-flex h-8 items-center gap-2 rounded-md border border-input bg-transparent px-3 text-xs font-medium hover:bg-accent transition-colors disabled:pointer-events-none disabled:opacity-50"
            >
              <Loader2 v-if="reconcileLoading" class="h-3.5 w-3.5 animate-spin" />
              对账状态
            </button>
            <button
              :disabled="reconcileLoading"
              @click="handleReconcile(true)"
              class="inline-flex h-8 items-center gap-2 rounded-md border border-amber-500/40 bg-amber-500/10 px-3 text-xs font-medium text-amber-700 hover:bg-amber-500/20 transition-colors disabled:pointer-events-none disabled:opacity-50 dark:text-amber-300"
            >
              清理残留进程
            </button>
          </div>
        </div>
        <div v-if="reconcileError" class="mt-3 rounded-md border border-destructive/40 bg-destructive/5 px-3 py-2 text-xs text-destructive">
          对账失败：{{ reconcileError }}
        </div>
        <div v-if="reconcileActions.length" class="mt-3 rounded-md border border-border/60 bg-background px-3 py-2 text-xs text-muted-foreground">
          <div v-for="action in reconcileActions" :key="action">{{ action }}</div>
        </div>
      </div>

      <div v-if="runtime?.entry_url" class="rounded-lg border border-border bg-card p-4">
        <div class="text-sm text-muted-foreground mb-2">访问地址</div>
        <div class="space-y-1">
          <div class="flex items-center gap-2">
            <span class="text-xs text-muted-foreground w-16 shrink-0">对外地址</span>
            <a :href="runtime.public_entry_url || runtime.entry_url" target="_blank" class="text-sm text-primary hover:underline">
              {{ runtime.public_entry_url || runtime.entry_url }}
            </a>
            <button @click="copyText(runtime.public_entry_url || runtime.entry_url || '')"
              class="text-xs text-muted-foreground hover:text-foreground transition-colors">复制</button>
          </div>
          <div v-if="runtime.local_entry_url && runtime.local_entry_url !== runtime.entry_url" class="flex items-center gap-2">
            <span class="text-xs text-muted-foreground w-16 shrink-0">本机调试</span>
            <a :href="runtime.local_entry_url" target="_blank" class="text-sm text-muted-foreground hover:underline">
              {{ runtime.local_entry_url }}
            </a>
          </div>
          <div v-if="!runtime.public_entry_url" class="text-xs text-amber-600 mt-1">仅本机地址，未配置 public_base_url</div>
          <div v-if="runtime.viewer_url" class="flex items-center gap-2 pt-2">
            <span class="text-xs text-muted-foreground w-16 shrink-0">Viewer</span>
            <a :href="runtime.viewer_url" target="_blank" class="text-sm text-primary hover:underline">
              {{ runtime.viewer_url }}
            </a>
            <button @click="copyText(runtime.viewer_url || '')"
              class="text-xs text-muted-foreground hover:text-foreground transition-colors">复制</button>
          </div>
        </div>
      </div>

      <div
        v-if="logsError"
        class="rounded-lg border border-amber-500/40 bg-amber-500/5 px-4 py-3 text-sm text-amber-700 dark:text-amber-300"
      >
        日志加载失败：{{ logsError }}
      </div>

      <SiteLogSummaryPanel v-if="logsData?.streams" :streams="logsData.streams" />

      <div class="rounded-lg border border-border bg-card">
        <div class="flex flex-wrap items-center justify-between gap-2 border-b border-border px-4 py-2">
          <div class="flex flex-wrap items-center gap-2">
            <button
              v-for="tab in (['parse', 'generate', 'db', 'web', 'viewer'] as const)"
              :key="tab"
              @click="onLogTabChange(tab)"
              class="rounded-md px-3 py-1 text-xs font-medium transition-colors"
              :class="activeLogTab === tab ? 'bg-accent text-accent-foreground' : 'text-muted-foreground hover:text-foreground'"
            >
              {{ logTabLabel(tab) }}
            </button>
            <span class="text-xs text-muted-foreground">
              {{ selectedLogState.loading
                ? '加载中...'
                : `显示 ${selectedLogState.lines.length} / ${selectedLogState.total} 行（limit ${selectedLogState.limit}）` }}
            </span>
          </div>
          <div class="flex items-center gap-2">
            <button
              v-if="selectedLogState.truncated && selectedLogState.limit < 5000"
              :disabled="selectedLogState.loading"
              @click="loadMoreLog"
              class="inline-flex h-7 items-center rounded-md border border-input bg-transparent px-3 text-xs font-medium hover:bg-accent transition-colors disabled:pointer-events-none disabled:opacity-50"
            >
              加载更多
            </button>
            <button
              :disabled="downloadPending || selectedLogState.total === 0"
              @click="downloadLog"
              class="inline-flex h-7 items-center rounded-md border border-input bg-transparent px-3 text-xs font-medium hover:bg-accent transition-colors disabled:pointer-events-none disabled:opacity-50"
            >
              {{ downloadPending ? '下载中...' : '下载完整日志' }}
            </button>
          </div>
        </div>
        <div v-if="downloadError" class="border-b border-destructive/40 bg-destructive/5 px-4 py-2 text-xs text-destructive">
          {{ downloadError }}
        </div>
        <div class="max-h-80 overflow-auto p-4">
          <div v-if="selectedLogs.length === 0 && !selectedLogState.loading" class="text-sm text-muted-foreground text-center py-4">暂无日志</div>
          <div v-else class="font-mono text-xs leading-relaxed space-y-0.5">
            <div v-for="(line, i) in selectedLogs" :key="i" class="whitespace-pre-wrap break-all">{{ line }}</div>
          </div>
        </div>
      </div>
    </div>

    <div v-else-if="site" class="space-y-4">
      <div class="rounded-lg border border-border bg-card p-5">
        <div class="mb-4 flex flex-wrap items-center justify-between gap-3">
          <div class="flex items-center gap-2">
            <ListChecks class="h-4 w-4 text-muted-foreground" />
            <h3 class="text-base font-medium">一键部署进度</h3>
          </div>
          <button
            :disabled="preflightLoading"
            @click="fetchPreflight"
            class="inline-flex h-8 items-center gap-2 rounded-md border border-input bg-transparent px-3 text-xs font-medium hover:bg-accent transition-colors disabled:pointer-events-none disabled:opacity-50"
          >
            <Loader2 v-if="preflightLoading" class="h-3.5 w-3.5 animate-spin" />
            {{ preflightLoading ? '检查中...' : '刷新预检' }}
          </button>
        </div>

        <div
          v-if="preflightError"
          class="mb-4 rounded-lg border border-destructive/50 bg-destructive/5 px-4 py-3 text-sm text-destructive"
        >
          预检失败：{{ preflightError }}
        </div>

        <div
          v-if="deployTaskId"
          class="mb-4 rounded-lg border border-border bg-background/60 p-4"
        >
          <div class="flex flex-wrap items-center justify-between gap-3">
            <div>
              <div class="flex flex-wrap items-center gap-2">
                <span class="text-sm font-medium">当前部署任务</span>
                <span
                  v-if="deployTask"
                  class="inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium"
                  :class="taskStatusConfig[deployTask.status]?.class"
                >
                  {{ taskStatusConfig[deployTask.status]?.label ?? deployTask.status }}
                </span>
              </div>
              <div class="mt-1 font-mono text-xs text-muted-foreground break-all">{{ deployTaskId }}</div>
            </div>
            <div class="flex items-center gap-2">
              <router-link
                :to="{ name: 'task-detail', params: { id: deployTaskId } }"
                class="inline-flex h-8 items-center rounded-md border border-input bg-transparent px-3 text-xs font-medium hover:bg-accent transition-colors"
              >
                打开任务
              </router-link>
              <button
                :disabled="deployTaskLoading"
                @click="fetchDeployTask"
                class="inline-flex h-8 items-center gap-2 rounded-md border border-input bg-transparent px-3 text-xs font-medium hover:bg-accent transition-colors disabled:pointer-events-none disabled:opacity-50"
              >
                <Loader2 v-if="deployTaskLoading" class="h-3.5 w-3.5 animate-spin" />
                {{ deployTaskLoading ? '刷新中...' : '刷新任务' }}
              </button>
            </div>
          </div>

          <div v-if="deployTask" class="mt-4 space-y-2">
            <div class="flex items-center justify-between text-xs text-muted-foreground">
              <span>{{ deployTask.progress.current_step }}</span>
              <span>{{ deployTaskPercent }}%</span>
            </div>
            <div class="h-2 overflow-hidden rounded-full bg-muted">
              <div class="h-full rounded-full bg-primary transition-all" :style="{ width: `${deployTaskPercent}%` }" />
            </div>
            <div
              v-if="deployTask.error"
              class="rounded-md border border-destructive/40 bg-destructive/5 px-3 py-2 text-xs text-destructive"
            >
              {{ deployTask.error }}
            </div>
          </div>
          <div
            v-else-if="deployTaskError"
            class="mt-3 rounded-md border border-amber-500/40 bg-amber-500/5 px-3 py-2 text-xs text-amber-700 dark:text-amber-300"
          >
            任务加载失败：{{ deployTaskError }}
          </div>
        </div>

        <div class="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
          <div
            v-for="step in deployProgressSteps"
            :key="step.key"
            class="rounded-lg border p-4"
            :class="deployStepClass(step)"
          >
            <div class="flex items-center gap-2 text-sm font-medium">
              <component
                :is="deployStepIcon(step)"
                class="h-4 w-4"
                :class="step.state === 'current' ? 'animate-spin' : ''"
              />
              <span>{{ step.label }}</span>
            </div>
            <div class="mt-2 text-xs opacity-80">{{ step.detail }}</div>
          </div>
        </div>
      </div>

      <div class="rounded-lg border border-border bg-card p-5">
        <div class="mb-4 flex flex-wrap items-center justify-between gap-3">
          <div>
            <h3 class="text-base font-medium">远端 Linux 部署</h3>
            <p class="mt-1 text-sm text-muted-foreground">
              上传当前站点 RocksDB 到 Linux，并启动远端 SurrealDB + web_server。
            </p>
          </div>
          <div class="flex items-center gap-2">
            <button
              :disabled="remotePreflightLoading || remoteDeployLoading"
              @click="handleRemotePreflight"
              class="inline-flex h-8 items-center gap-2 rounded-md border border-input bg-transparent px-3 text-xs font-medium hover:bg-accent transition-colors disabled:pointer-events-none disabled:opacity-50"
            >
              <Loader2 v-if="remotePreflightLoading" class="h-3.5 w-3.5 animate-spin" />
              {{ remotePreflightLoading ? '检查中...' : '远端预检' }}
            </button>
            <button
              :disabled="remoteDeployLoading || remotePreflightLoading"
              @click="handleRemoteDeploy"
              class="inline-flex h-8 items-center gap-2 rounded-md bg-primary px-3 text-xs font-medium text-primary-foreground hover:bg-primary/90 transition-colors disabled:pointer-events-none disabled:opacity-50"
            >
              <Loader2 v-if="remoteDeployLoading" class="h-3.5 w-3.5 animate-spin" />
              {{ remoteDeployLoading ? '提交中...' : '远端一键部署' }}
            </button>
          </div>
        </div>

        <div
          v-if="remoteDeployError"
          class="mb-4 rounded-lg border border-destructive/50 bg-destructive/5 px-4 py-3 text-sm text-destructive"
        >
          远端部署失败：{{ remoteDeployError }}
        </div>

        <div class="grid gap-3 md:grid-cols-2 xl:grid-cols-4">
          <label class="space-y-1 text-xs text-muted-foreground">
            <span>主机</span>
            <input v-model="remoteTargetForm.host" class="h-9 w-full rounded-md border border-input bg-background px-3 text-sm text-foreground" />
          </label>
          <label class="space-y-1 text-xs text-muted-foreground">
            <span>SSH 用户</span>
            <input v-model="remoteTargetForm.ssh_user" class="h-9 w-full rounded-md border border-input bg-background px-3 text-sm text-foreground" />
          </label>
          <label class="space-y-1 text-xs text-muted-foreground">
            <span>密码环境变量</span>
            <input v-model="remoteTargetForm.password_env" class="h-9 w-full rounded-md border border-input bg-background px-3 text-sm text-foreground" />
          </label>
          <label class="space-y-1 text-xs text-muted-foreground">
            <span>Surreal 路径</span>
            <input v-model="remoteTargetForm.surreal_bin" class="h-9 w-full rounded-md border border-input bg-background px-3 text-sm text-foreground" />
          </label>
          <label class="space-y-1 text-xs text-muted-foreground">
            <span>Web Server 路径</span>
            <input v-model="remoteTargetForm.remote_web_bin" class="h-9 w-full rounded-md border border-input bg-background px-3 text-sm text-foreground" />
          </label>
          <label class="space-y-1 text-xs text-muted-foreground">
            <span>远端根目录</span>
            <input v-model="remoteTargetForm.remote_root" class="h-9 w-full rounded-md border border-input bg-background px-3 text-sm text-foreground" />
          </label>
          <label class="space-y-1 text-xs text-muted-foreground">
            <span>远端 DB 路径</span>
            <input v-model="remoteTargetForm.remote_db_path" :placeholder="`/root/surreal_data/${site.site_id}.db`" class="h-9 w-full rounded-md border border-input bg-background px-3 text-sm text-foreground" />
          </label>
          <label class="space-y-1 text-xs text-muted-foreground">
            <span>DB 端口</span>
            <input v-model.number="remoteTargetForm.remote_db_port" type="number" class="h-9 w-full rounded-md border border-input bg-background px-3 text-sm text-foreground" />
          </label>
          <label class="space-y-1 text-xs text-muted-foreground">
            <span>Web 端口</span>
            <input v-model.number="remoteTargetForm.remote_web_port" type="number" class="h-9 w-full rounded-md border border-input bg-background px-3 text-sm text-foreground" />
          </label>
        </div>

        <div v-if="remoteDeployStatus" class="mt-4 rounded-lg border border-border bg-background/60 p-4 text-sm">
          <div class="flex flex-wrap items-center justify-between gap-3">
            <div>
              <div class="font-medium">状态：{{ remoteDeployStatus.status }} / {{ remoteDeployStatus.current_step }}</div>
              <a
                v-if="remoteDeployStatus.remote_entry_url"
                :href="remoteDeployStatus.remote_entry_url"
                target="_blank"
                class="mt-1 block text-xs text-primary hover:underline break-all"
              >
                {{ remoteDeployStatus.remote_entry_url }}
              </a>
            </div>
            <span
              class="inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium"
              :class="remoteBlockingCount > 0 ? 'bg-red-100 text-red-800 dark:bg-red-900 dark:text-red-200' : remoteWarningCount > 0 ? 'bg-amber-100 text-amber-800 dark:bg-amber-900 dark:text-amber-200' : 'bg-emerald-100 text-emerald-800 dark:bg-emerald-900 dark:text-emerald-200'"
            >
              {{ remoteBlockingCount }} 阻断 / {{ remoteWarningCount }} 警告
            </span>
          </div>
          <div v-if="remoteDeployStatus.checks.length" class="mt-3 grid gap-2 md:grid-cols-2">
            <div
              v-for="check in remoteDeployStatus.checks"
              :key="check.key"
              class="rounded-md border p-3 text-xs"
              :class="preflightCheckClass(check)"
            >
              <div class="font-medium">{{ check.label }} · {{ preflightStatusLabel(check.status) }}</div>
              <div class="mt-1">{{ check.message }}</div>
              <div v-if="check.detail" class="mt-1 text-muted-foreground break-all">{{ check.detail }}</div>
            </div>
          </div>
        </div>
      </div>

      <div class="rounded-lg border border-border bg-card p-5">
        <div class="mb-4 flex items-center justify-between gap-3">
          <div>
            <h3 class="text-base font-medium">部署后验收</h3>
            <p class="mt-1 text-sm text-muted-foreground">
              {{ deployValidation?.exists
                ? `检查时间 ${formatDateTime(deployValidation.checked_at)}，${deployValidation.blocking_count} 个阻断 / ${deployValidation.warning_count} 个警告`
                : '部署成功后会生成验收报告，覆盖 Web、Viewer、Parquet 和 GLB 资源。' }}
            </p>
          </div>
          <button
            :disabled="deployValidationLoading"
            @click="refreshDeployValidation"
            class="inline-flex h-8 items-center gap-2 rounded-md border border-input bg-transparent px-3 text-xs font-medium hover:bg-accent transition-colors disabled:pointer-events-none disabled:opacity-50"
          >
            <Loader2 v-if="deployValidationLoading" class="h-3.5 w-3.5 animate-spin" />
            {{ deployValidationLoading ? '加载中...' : '刷新验收' }}
          </button>
        </div>

        <div
          v-if="deployValidationError"
          class="mb-4 rounded-lg border border-destructive/50 bg-destructive/5 px-4 py-3 text-sm text-destructive"
        >
          验收报告加载失败：{{ deployValidationError }}
        </div>

        <div v-if="deployValidation?.exists && deployValidation.checks.length" class="space-y-3">
          <div
            v-for="check in deployValidation.checks"
            :key="check.key"
            class="rounded-lg border p-4 text-sm"
            :class="deployValidationCheckClass(check)"
          >
            <div class="flex flex-wrap items-center justify-between gap-3">
              <div class="font-medium">{{ check.label }}</div>
              <span class="rounded-full border border-current/20 px-2 py-0.5 text-xs">
                {{ deployValidationStatusLabel(check.status) }}
              </span>
            </div>
            <div class="mt-2">{{ check.message }}</div>
            <div v-if="check.detail" class="mt-1 text-xs text-muted-foreground break-all">{{ check.detail }}</div>
            <a
              v-if="check.url"
              :href="check.url"
              target="_blank"
              class="mt-2 block text-xs text-primary hover:underline break-all"
            >
              {{ check.url }}
            </a>
          </div>
        </div>
        <div v-else class="rounded-lg border border-dashed border-border p-6 text-center text-sm text-muted-foreground">
          暂无部署后验收报告。提交完整部署并等待任务完成后再刷新。
        </div>
      </div>

      <div class="rounded-lg border border-border bg-card p-5">
        <div class="mb-4 flex items-center justify-between gap-3">
          <div>
            <h3 class="text-base font-medium">部署预检</h3>
            <p class="mt-1 text-sm text-muted-foreground">
              {{ preflight
                ? `更新时间 ${formatDateTime(preflight.updated_at)}，${preflight.blocking_count} 个阻断 / ${preflight.warning_count} 个警告`
                : '尚未执行预检' }}
            </p>
          </div>
          <span
            v-if="preflight"
            class="inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium"
            :class="preflight.ready ? 'bg-emerald-100 text-emerald-800 dark:bg-emerald-900 dark:text-emerald-200' : 'bg-red-100 text-red-800 dark:bg-red-900 dark:text-red-200'"
          >
            {{ preflight.ready ? '可以部署' : '存在阻断' }}
          </span>
        </div>

        <div v-if="preflight?.checks.length" class="space-y-3">
          <div
            v-for="check in preflight.checks"
            :key="check.key"
            class="rounded-lg border p-4 text-sm"
            :class="preflightCheckClass(check)"
          >
            <div class="flex flex-wrap items-center justify-between gap-3">
              <div class="font-medium">{{ check.label }}</div>
              <span class="rounded-full border border-current/20 px-2 py-0.5 text-xs">
                {{ preflightStatusLabel(check.status) }}
              </span>
            </div>
            <div class="mt-2">{{ check.message }}</div>
            <div v-if="check.detail" class="mt-1 text-xs text-muted-foreground break-all">{{ check.detail }}</div>
            <div v-if="check.action_hint" class="mt-2 text-xs text-muted-foreground">建议：{{ check.action_hint }}</div>
          </div>
        </div>
        <div v-else class="rounded-lg border border-dashed border-border p-6 text-center text-sm text-muted-foreground">
          点击“刷新预检”检查部署前置条件。
        </div>
      </div>

      <SiteConfigSections :site="site" />
    </div>

    <SiteDrawer
      :open="drawerOpen"
      :site-id="siteId"
      @close="drawerOpen = false"
      @saved="handleDrawerSaved"
    />
  </div>
</template>
