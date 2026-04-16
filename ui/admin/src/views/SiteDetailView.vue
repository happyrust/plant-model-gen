<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import {
  AlertTriangle,
  Database,
  FolderArchive,
  Globe,
  HardDrive,
  ShieldAlert,
  TimerReset,
} from 'lucide-vue-next'
import { sitesApi } from '@/api/sites'
import { usePolling } from '@/composables/usePolling'
import { useSitesStore } from '@/stores/sites'
import SiteDetailHeader from '@/components/sites/SiteDetailHeader.vue'
import SiteRuntimeCards from '@/components/sites/SiteRuntimeCards.vue'
import SiteLogSummaryPanel from '@/components/sites/SiteLogSummaryPanel.vue'
import SiteConfigSections from '@/components/sites/SiteConfigSections.vue'
import type {
  ManagedProjectSite,
  ManagedSiteLogsResponse,
  ManagedSiteProcessResource,
  ManagedSiteRiskLevel,
  ManagedSiteRuntimeStatus,
} from '@/types/site'

const route = useRoute()
const router = useRouter()
const sitesStore = useSitesStore()

const site = ref<ManagedProjectSite | null>(null)
const runtime = ref<ManagedSiteRuntimeStatus | null>(null)
const logsData = ref<ManagedSiteLogsResponse | null>(null)
const activeTab = ref<'overview' | 'deploy'>('overview')
const activeLogTab = ref<'parse' | 'db' | 'web'>('parse')

const siteId = computed(() => String(route.params.id ?? ''))
const resources = computed(() => runtime.value?.resources ?? null)

const selectedLogs = computed(() => {
  if (logsData.value === null) return []
  if (activeLogTab.value === 'parse') return logsData.value.parse_log
  if (activeLogTab.value === 'db') return logsData.value.db_log
  return logsData.value.web_log
})

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
    key: 'parse',
    label: 'Parse 进程',
    icon: TimerReset,
    process: resources.value?.parse_process ?? null,
  },
])

const riskTone = computed(() => toneForRisk(runtime.value?.risk_level ?? 'normal'))
const parseHealthTone = computed(() => {
  const status = runtime.value?.parse_health.status ?? 'unknown'
  if (status === 'critical') return 'text-red-700 dark:text-red-300'
  if (status === 'warning') return 'text-amber-700 dark:text-amber-300'
  if (status === 'normal') return 'text-emerald-700 dark:text-emerald-300'
  return 'text-muted-foreground'
})

async function fetchAll() {
  const id = siteId.value
  try {
    site.value = await sitesApi.get(id)
    runtime.value = await sitesApi.runtime(id)
    logsData.value = await sitesApi.logs(id)
  } catch {
    // partial failure is acceptable
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

function viewerUrl() {
  const s = site.value
  if (!s?.web_port) return null
  const project = encodeURIComponent(s.associated_project || s.project_name)
  return `http://localhost:3101/?backendPort=${s.web_port}&output_project=${project}`
}

function openViewer() {
  const url = viewerUrl()
  if (url) window.open(url, '_blank')
}

function copyText(text: string) {
  navigator.clipboard.writeText(text)
}

const { start: startPolling } = usePolling(fetchAll, 10000)

onMounted(async () => {
  await fetchAll()
  startPolling()
})
</script>

<template>
  <div class="space-y-6">
    <SiteDetailHeader
      :site="site"
      :viewer-url="viewerUrl()"
      @back="router.push({ path: '/sites' })"
      @start="sitesStore.startSite(siteId).then(fetchAll)"
      @stop="sitesStore.stopSite(siteId).then(fetchAll)"
      @parse="sitesStore.parseSite(siteId).then(fetchAll)"
      @refresh="fetchAll"
      @open-viewer="openViewer()"
    />

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
      >配置信息</button>
    </div>

    <div v-if="activeTab === 'overview'" class="space-y-4">
      <SiteRuntimeCards :site="site" :runtime="runtime" />

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
        <div class="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
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

      <div v-if="runtime?.db_port_conflict || runtime?.web_port_conflict" class="rounded-lg border border-amber-500/50 bg-amber-500/5 p-4">
        <div class="text-sm font-medium text-amber-700 dark:text-amber-300 mb-1">端口冲突</div>
        <div v-if="runtime?.web_port_conflict" class="text-sm text-amber-600 dark:text-amber-400">
          Web 端口 {{ runtime.web_port }} 被外部进程占用 (PIDs: {{ runtime.web_conflict_pids?.join(', ') }})
        </div>
        <div v-if="runtime?.db_port_conflict" class="text-sm text-amber-600 dark:text-amber-400">
          DB 端口 {{ runtime.db_port }} 被外部进程占用 (PIDs: {{ runtime.db_conflict_pids?.join(', ') }})
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
        </div>
      </div>

      <SiteLogSummaryPanel v-if="logsData?.streams" :streams="logsData.streams" />

      <div class="rounded-lg border border-border bg-card">
        <div class="flex items-center gap-2 border-b border-border px-4 py-2">
          <button
            v-for="tab in (['parse', 'db', 'web'] as const)"
            :key="tab"
            @click="activeLogTab = tab"
            class="rounded-md px-3 py-1 text-xs font-medium transition-colors"
            :class="activeLogTab === tab ? 'bg-accent text-accent-foreground' : 'text-muted-foreground hover:text-foreground'"
          >
            {{ tab === 'parse' ? '解析日志' : tab === 'db' ? 'DB 日志' : 'Web 日志' }}
          </button>
        </div>
        <div class="max-h-80 overflow-auto p-4">
          <div v-if="selectedLogs.length === 0" class="text-sm text-muted-foreground text-center py-4">暂无日志</div>
          <div v-else class="font-mono text-xs leading-relaxed space-y-0.5">
            <div v-for="(line, i) in selectedLogs" :key="i" class="whitespace-pre-wrap break-all">{{ line }}</div>
          </div>
        </div>
      </div>
    </div>

    <SiteConfigSections v-else-if="site" :site="site" />
  </div>
</template>
