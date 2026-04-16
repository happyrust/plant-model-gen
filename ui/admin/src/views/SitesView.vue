<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { AlertTriangle, Cpu, FolderKanban, HardDrive, MemoryStick, Server, Activity, CircleAlert } from 'lucide-vue-next'
import { sitesApi } from '@/api/sites'
import { usePolling } from '@/composables/usePolling'
import SiteDataTable from '@/components/sites/SiteDataTable.vue'
import SiteDrawer from '@/components/sites/SiteDrawer.vue'
import SiteToolbar from '@/components/sites/SiteToolbar.vue'
import SiteWorkbenchHeader from '@/components/sites/SiteWorkbenchHeader.vue'
import { useSitesStore } from '@/stores/sites'
import { matchesQuickFilter, computeStats, type QuickFilter } from '@/components/sites/site-status'
import type { AdminResourceSummary, ManagedSiteRiskLevel } from '@/types/site'

const sitesStore = useSitesStore()

const drawerOpen = ref(false)
const editingSiteId = ref<string | null>(null)
const searchQuery = ref('')
const statusFilter = ref('')
const riskFilter = ref<ManagedSiteRiskLevel | ''>('')
const activeQuickFilter = ref<QuickFilter>('all')
const lastRefresh = ref<string | null>(null)
const refreshing = ref(false)
const resourceSummary = ref<AdminResourceSummary | null>(null)
const resourceLoading = ref(false)
const resourceError = ref('')

const siteStats = computed(() => computeStats(sitesStore.sites))

const filteredSites = computed(() => {
  let list = sitesStore.sites
  if (activeQuickFilter.value !== 'all') {
    list = list.filter((s) => matchesQuickFilter(s, activeQuickFilter.value))
  }
  if (searchQuery.value) {
    const q = searchQuery.value.toLowerCase()
    list = list.filter((s) =>
      s.project_name.toLowerCase().includes(q) || s.site_id.toLowerCase().includes(q)
    )
  }
  if (statusFilter.value) {
    list = list.filter((s) => s.status === statusFilter.value)
  }
  if (riskFilter.value) {
    list = list.filter((s) => s.risk_level === riskFilter.value)
  }
  return list
})

const resourceCards = computed(() => [
  {
    key: 'cpu',
    label: 'CPU',
    value: formatPercent(resourceSummary.value?.cpu_usage),
    hint: '更新时间 ' + formatDateTime(resourceSummary.value?.updated_at),
    icon: Cpu,
  },
  {
    key: 'memory',
    label: '内存',
    value: formatPercent(resourceSummary.value?.memory_usage),
    hint: '当前机器内存占用',
    icon: MemoryStick,
  },
  {
    key: 'disk',
    label: '磁盘',
    value: formatPercent(resourceSummary.value?.disk_usage),
    hint: '当前运行目录所在磁盘',
    icon: HardDrive,
  },
  {
    key: 'managed-data',
    label: '管理数据目录大小',
    value: formatBytes(resourceSummary.value?.managed_data_size_bytes),
    hint: 'admin 运行目录 ' + formatBytes(resourceSummary.value?.admin_runtime_size_bytes),
    icon: FolderKanban,
  },
])

const resourceRiskBanner = computed(() => {
  if (resourceSummary.value === null && resourceError.value) {
    return {
      title: '机器资源读取失败',
      class: 'border-amber-500/40 bg-amber-500/5 text-amber-700 dark:text-amber-300',
      detail: resourceError.value,
    }
  }

  const level = resourceSummary.value?.risk_level ?? 'normal'
  if (level === 'critical') {
    return {
      title: '机器资源风险：严重',
      class: 'border-red-500/40 bg-red-500/5 text-red-700 dark:text-red-300',
      detail: resourceSummary.value?.warnings.join('；') || '请优先处理机器资源占用。',
    }
  }
  if (level === 'warning') {
    return {
      title: '机器资源风险：警告',
      class: 'border-amber-500/40 bg-amber-500/5 text-amber-700 dark:text-amber-300',
      detail: resourceSummary.value?.warnings.join('；') || '当前机器资源接近阈值。',
    }
  }
  return {
    title: '机器资源风险：正常',
    class: 'border-emerald-500/40 bg-emerald-500/5 text-emerald-700 dark:text-emerald-300',
    detail: '当前没有明显资源风险。',
  }
})

function openCreateDrawer() {
  editingSiteId.value = null
  drawerOpen.value = true
}

function handleFilter(search: string, status: string, risk: string) {
  searchQuery.value = search
  statusFilter.value = status
  riskFilter.value = (risk as ManagedSiteRiskLevel | '') || ''
}

function handleQuickFilter(filter: QuickFilter) {
  activeQuickFilter.value = filter
}

function handleDrawerSaved() {
  drawerOpen.value = false
  editingSiteId.value = null
  void fetchPageData()
}

function formatPercent(value?: number | null) {
  if (value === null || value === undefined || Number.isNaN(value)) return '-'
  return Math.round(value) + '%'
}

function formatBytes(value?: number | null) {
  if (!value || value <= 0) return '0 B'
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
  if (!value) return '-'
  const date = new Date(value)
  if (Number.isNaN(date.getTime())) return '-'
  return date.toLocaleString('zh-CN', {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
  })
}

async function fetchResourceSummary() {
  resourceLoading.value = true
  try {
    const summary = await sitesApi.resourceSummary()
    resourceSummary.value = summary
    resourceError.value = summary.message ?? ''
  } catch (err: unknown) {
    resourceError.value = err instanceof Error ? err.message : '获取资源摘要失败'
  } finally {
    resourceLoading.value = false
  }
}

async function fetchPageData() {
  refreshing.value = true
  await Promise.allSettled([sitesStore.fetchSites(), fetchResourceSummary()])
  lastRefresh.value = new Date().toISOString()
  refreshing.value = false
}

const { start: startPolling } = usePolling(async () => {
  await fetchPageData()
}, 30000)

onMounted(async () => {
  await fetchPageData()
  startPolling()
})
</script>

<template>
  <div class="space-y-6">
    <SiteWorkbenchHeader
      :total="siteStats.total"
      :filtered="filteredSites.length"
      :last-refresh="lastRefresh"
      :refreshing="refreshing"
      @refresh="fetchPageData"
    />
    <div class="grid gap-4 md:grid-cols-2 lg:grid-cols-4">
      <div class="rounded-lg border border-border bg-card p-4">
        <div class="flex items-center justify-between">
          <span class="text-sm text-muted-foreground">总站点</span>
          <Server class="h-4 w-4 text-muted-foreground" />
        </div>
        <div class="mt-2 text-2xl font-bold">{{ siteStats.total }}</div>
        <div class="mt-1 text-xs text-muted-foreground">当前结果 {{ filteredSites.length }} 条</div>
      </div>
      <div class="rounded-lg border border-border bg-card p-4">
        <div class="flex items-center justify-between">
          <span class="text-sm text-muted-foreground">运行中</span>
          <Activity class="h-4 w-4 text-green-600" />
        </div>
        <div class="mt-2 text-2xl font-bold text-green-600">{{ siteStats.running }}</div>
      </div>
      <div class="rounded-lg border border-border bg-card p-4">
        <div class="flex items-center justify-between">
          <span class="text-sm text-muted-foreground">处理中</span>
          <Cpu class="h-4 w-4 text-amber-600" />
        </div>
        <div class="mt-2 text-2xl font-bold text-amber-600">{{ siteStats.busy }}</div>
        <div class="mt-1 text-xs text-muted-foreground">启动/停止/解析中</div>
      </div>
      <div class="rounded-lg border border-border bg-card p-4">
        <div class="flex items-center justify-between">
          <span class="text-sm text-muted-foreground">异常</span>
          <CircleAlert class="h-4 w-4 text-destructive" />
        </div>
        <div class="mt-2 text-2xl font-bold text-destructive">{{ siteStats.error }}</div>
      </div>
    </div>

    <section class="space-y-3">
      <div class="flex items-center justify-between">
        <div>
          <h3 class="text-lg font-medium">机器资源概览</h3>
          <p class="text-sm text-muted-foreground">跟随列表页刷新节奏更新，不影响站点列表</p>
        </div>
        <span v-if="resourceLoading && resourceSummary === null" class="text-sm text-muted-foreground">加载中...</span>
      </div>

      <div class="rounded-lg border px-4 py-3" :class="resourceRiskBanner.class">
        <div class="flex items-start gap-3">
          <AlertTriangle class="mt-0.5 h-4 w-4" />
          <div>
            <div class="text-sm font-medium">{{ resourceRiskBanner.title }}</div>
            <div class="mt-1 text-sm">{{ resourceRiskBanner.detail }}</div>
          </div>
        </div>
      </div>

      <div v-if="resourceError" class="rounded-lg border border-amber-500/40 bg-amber-500/5 px-4 py-3 text-sm text-amber-700 dark:text-amber-300">
        {{ resourceError }}
      </div>

      <div class="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
        <div
          v-for="card in resourceCards"
          :key="card.key"
          class="rounded-lg border border-border bg-card p-4 transition-colors hover:bg-accent/30"
        >
          <div class="flex items-center justify-between">
            <span class="text-sm text-muted-foreground">{{ card.label }}</span>
            <component :is="card.icon" class="h-4 w-4 text-muted-foreground" />
          </div>
          <div class="mt-2 text-2xl font-bold">{{ card.value }}</div>
          <div class="mt-1 text-xs text-muted-foreground">{{ card.hint }}</div>
        </div>
      </div>
    </section>

    <SiteToolbar @open-drawer="openCreateDrawer" @filter="handleFilter" @quick-filter="handleQuickFilter" />
    <SiteDataTable :sites="filteredSites" :loading="sitesStore.loading" />
    <SiteDrawer
      :open="drawerOpen"
      :site-id="editingSiteId"
      @close="drawerOpen = false"
      @saved="handleDrawerSaved"
    />
  </div>
</template>
