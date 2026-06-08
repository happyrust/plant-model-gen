<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { AlertTriangle, ChevronLeft, ChevronRight, Database, ExternalLink, RefreshCw, Search, Table2 } from 'lucide-vue-next'
import { extractErrorMessage } from '@/api/client'
import { dataBrowserApi, type DataBrowserConnectionMode, type DataBrowserConnectionResponse, type DataBrowserRecordsResponse, type DataBrowserTable } from '@/api/data-browser'
import { isLoopbackHost, isWriteSurrealQuery, SiteSurrealBrowserClient, type SurrealQueryResponse } from '@/lib/data-browser-surreal'
import { useSitesStore } from '@/stores/sites'
import type { ManagedProjectSite } from '@/types/site'

const SELECTED_SITE_STORAGE_KEY = 'admin:data-browser:selected-site-id'

const route = useRoute()
const router = useRouter()
const sitesStore = useSitesStore()
const tables = ref<DataBrowserTable[]>([])
const activeTable = ref('')
const tableSearch = ref('')
const loadingTables = ref(false)
const loadingRecords = ref(false)
const error = ref('')
const recordsResult = ref<DataBrowserRecordsResponse | null>(null)
const page = ref(1)
const perPage = ref(25)
const sort = ref('id')
const dir = ref<'asc' | 'desc'>('asc')
const selectedSiteId = ref('')
const siteSelectionInitialized = ref(false)
const browserClient = new SiteSurrealBrowserClient()
const connection = ref<DataBrowserConnectionResponse | null>(null)
const connectionStatus = ref<'idle' | 'connecting' | 'connected'>('idle')
const querySql = ref('INFO FOR DB;')
const queryRunning = ref(false)
const queryResult = ref<SurrealQueryResponse[] | null>(null)
const writeMode = ref(false)

const selectedSite = computed(() =>
  sitesStore.sites.find((site) => site.site_id === selectedSiteId.value) ?? null,
)

const selectedSiteBlockReason = computed(() => {
  if (!selectedSiteId.value) return '请选择要浏览的站点数据库。'
  if (!selectedSite.value) return '当前 URL 指定的站点不存在或已被删除。'
  return siteBrowseBlockReason(selectedSite.value)
})

const selectedSiteCanBrowse = computed(() =>
  Boolean(selectedSite.value && !siteBrowseBlockReason(selectedSite.value)),
)

const directBrowserBlockReason = computed(() => {
  if (!selectedSiteCanBrowse.value) return selectedSiteBlockReason.value
  if (isLoopbackHost()) return '当前页面通过 loopback 地址访问，已禁用；请使用服务器真实 IP 打开 Admin。'
  return ''
})

const canUseDirectBrowser = computed(() => !directBrowserBlockReason.value)

const selectedSiteLabel = computed(() =>
  selectedSite.value ? siteDisplayName(selectedSite.value) : '未选择站点',
)

const queryNeedsWriteMode = computed(() => isWriteSurrealQuery(querySql.value))

const filteredTables = computed(() => {
  const keyword = tableSearch.value.trim().toLowerCase()
  if (!keyword) return tables.value
  return tables.value.filter((table) => table.name.toLowerCase().includes(keyword))
})

const totalPages = computed(() => {
  const total = recordsResult.value?.total ?? 0
  return Math.max(1, Math.ceil(total / perPage.value))
})

const pageStart = computed(() => {
  const total = recordsResult.value?.total ?? 0
  if (!total) return 0
  return (page.value - 1) * perPage.value + 1
})

const pageEnd = computed(() => {
  const total = recordsResult.value?.total ?? 0
  return Math.min(total, page.value * perPage.value)
})

function stringQueryParam(value: unknown): string {
  if (typeof value === 'string') return value
  if (Array.isArray(value) && typeof value[0] === 'string') return value[0]
  return ''
}

function siteDisplayName(site: ManagedProjectSite): string {
  return site.site_name || site.project_name || site.site_id
}

function siteBrowseBlockReason(site: ManagedProjectSite): string {
  if (site.status !== 'Running') return `站点未运行（当前 ${site.status}）`
  if (site.runtime_db_mode !== 'ws') return `运行库模式为 ${site.runtime_db_mode}，当前仅支持 ws`
  if (!site.db_port) return '缺少运行库端口'
  return ''
}

function clearBrowserState() {
  tables.value = []
  activeTable.value = ''
  tableSearch.value = ''
  recordsResult.value = null
  queryResult.value = null
  writeMode.value = false
  connection.value = null
  connectionStatus.value = 'idle'
  page.value = 1
  sort.value = 'id'
  dir.value = 'asc'
}

async function closeBrowserConnection() {
  await browserClient.close()
  connection.value = null
  connectionStatus.value = 'idle'
}

function rememberSelectedSite(siteId: string) {
  if (siteId) {
    localStorage.setItem(SELECTED_SITE_STORAGE_KEY, siteId)
  } else {
    localStorage.removeItem(SELECTED_SITE_STORAGE_KEY)
  }
}

function updateRouteSiteId(siteId: string) {
  const nextQuery = { ...route.query }
  if (siteId) {
    nextQuery.site_id = siteId
  } else {
    delete nextQuery.site_id
  }
  if (stringQueryParam(route.query.site_id) === siteId) return
  void router.replace({ query: nextQuery })
}

function setSelectedSite(siteId: string, options: { persist?: boolean; syncRoute?: boolean } = {}) {
  const { persist = true, syncRoute = true } = options
  if (selectedSiteId.value !== siteId) {
    void closeBrowserConnection()
    selectedSiteId.value = siteId
    clearBrowserState()
  }
  if (persist) rememberSelectedSite(siteId)
  if (syncRoute) updateRouteSiteId(siteId)
}

function siteExists(siteId: string): boolean {
  return sitesStore.sites.some((site) => site.site_id === siteId)
}

function getStoredSiteId(): string {
  try {
    return localStorage.getItem(SELECTED_SITE_STORAGE_KEY) ?? ''
  } catch {
    return ''
  }
}

function chooseInitialSiteId(): string {
  const querySiteId = stringQueryParam(route.query.site_id)
  if (querySiteId && siteExists(querySiteId)) return querySiteId

  const storedSiteId = getStoredSiteId()
  if (storedSiteId && siteExists(storedSiteId)) return storedSiteId

  const runningSites = sitesStore.sites.filter((site) => !siteBrowseBlockReason(site))
  if (runningSites.length === 1) return runningSites[0].site_id

  return ''
}

async function fetchTables() {
  if (!canUseDirectBrowser.value) {
    clearBrowserState()
    return
  }
  loadingTables.value = true
  error.value = ''
  const siteId = selectedSiteId.value
  try {
    await connectBrowser('reader')
    const result = await browserClient.listTables()
    if (selectedSiteId.value !== siteId) return
    tables.value = result
    if (!result.some((table) => table.name === activeTable.value)) {
      activeTable.value = ''
    }
    if (!activeTable.value && result.length > 0) {
      activeTable.value = result[0].name
    }
    if (!activeTable.value) recordsResult.value = null
  } catch (err: unknown) {
    error.value = extractErrorMessage(err)
    clearBrowserState()
  } finally {
    loadingTables.value = false
  }
}

async function fetchRecords() {
  if (!canUseDirectBrowser.value || !activeTable.value) {
    recordsResult.value = null
    return
  }
  loadingRecords.value = true
  error.value = ''
  const siteId = selectedSiteId.value
  const tableName = activeTable.value
  try {
    await connectBrowser('reader')
    const result = await browserClient.fetchRecords(tableName, {
      page: page.value,
      per_page: perPage.value,
      sort: sort.value,
      dir: dir.value,
    })
    if (selectedSiteId.value !== siteId || activeTable.value !== tableName) return
    recordsResult.value = result
  } catch (err: unknown) {
    error.value = extractErrorMessage(err)
    recordsResult.value = null
  } finally {
    loadingRecords.value = false
  }
}

async function refreshBrowser() {
  await fetchTables()
  await fetchRecords()
}

async function connectBrowser(mode: DataBrowserConnectionMode) {
  const siteId = selectedSiteId.value
  if (!siteId) throw new Error('请选择要浏览的站点数据库。')
  if (!canUseDirectBrowser.value) throw new Error(directBrowserBlockReason.value)
  if (
    browserClient.isConnected
    && connection.value?.site.site_id === siteId
    && connection.value.mode === mode
  ) {
    return
  }
  connectionStatus.value = 'connecting'
  try {
    const nextConnection = await dataBrowserApi.connectionForSite(siteId, mode)
    if (selectedSiteId.value !== siteId) {
      connectionStatus.value = 'idle'
      return
    }
    await browserClient.connect(nextConnection)
    connection.value = nextConnection
    connectionStatus.value = 'connected'
  } catch (err) {
    connectionStatus.value = 'idle'
    connection.value = null
    await browserClient.close()
    throw err
  }
}

async function runSqlQuery() {
  const sql = querySql.value.trim()
  if (!sql) {
    error.value = '请输入 SurrealQL。'
    return
  }
  const isWriteQuery = isWriteSurrealQuery(sql)
  if (isWriteQuery && !writeMode.value) {
    error.value = '检测到写操作。请先开启写入模式，再执行该 SurrealQL。'
    return
  }
  if (isWriteQuery && !window.confirm('将使用 editor 凭据执行写操作。请确认你已备份或理解此变更。')) {
    return
  }
  queryRunning.value = true
  error.value = ''
  try {
    await connectBrowser(isWriteQuery ? 'editor' : 'reader')
    queryResult.value = await browserClient.runQuery(sql)
    if (isWriteQuery) await refreshBrowser()
  } catch (err: unknown) {
    error.value = extractErrorMessage(err)
    queryResult.value = null
  } finally {
    queryRunning.value = false
  }
}

async function refreshSites() {
  await sitesStore.fetchSites()
  if (selectedSiteId.value && !siteExists(selectedSiteId.value)) {
    setSelectedSite('')
    return
  }
  await fetchTables()
}

async function handleSiteSelect(event: Event) {
  const target = event.target as HTMLSelectElement
  setSelectedSite(target.value)
  await fetchTables()
}

function selectTable(name: string) {
  if (activeTable.value === name) return
  activeTable.value = name
  page.value = 1
  sort.value = 'id'
  dir.value = 'asc'
}

function toggleSort(column: string) {
  if (sort.value === column) {
    dir.value = dir.value === 'asc' ? 'desc' : 'asc'
  } else {
    sort.value = column
    dir.value = 'asc'
  }
  page.value = 1
}

function prevPage() {
  if (page.value > 1) page.value -= 1
}

function nextPage() {
  if (page.value < totalPages.value) page.value += 1
}

function formatCell(value: unknown): string {
  if (value === null || typeof value === 'undefined') return '—'
  if (typeof value === 'boolean') return value ? 'true' : 'false'
  if (typeof value === 'number') return Number.isFinite(value) ? String(value) : '—'
  if (typeof value === 'string') return value
  if (typeof value === 'object') {
    const object = value as Record<string, unknown>
    if (typeof object.tb === 'string' && typeof object.id !== 'undefined') {
      return `${object.tb}:${String(object.id)}`
    }
    if (typeof object.String === 'string') return object.String
    try {
      return JSON.stringify(value)
    } catch {
      return String(value)
    }
  }
  return String(value)
}

function cellTitle(value: unknown): string {
  return formatCell(value)
}

function isRecordLink(value: unknown): boolean {
  const formatted = formatCell(value)
  return /^[A-Za-z_][A-Za-z0-9_]*:.+/.test(formatted)
}

function formatJson(value: unknown): string {
  try {
    return JSON.stringify(value, null, 2)
  } catch {
    return String(value)
  }
}

function queryResponseTitle(response: SurrealQueryResponse, index: number): string {
  return response.success ? `结果 ${index + 1}` : `错误 ${index + 1}`
}

watch([activeTable, page, perPage, sort, dir], () => {
  void fetchRecords()
})

watch(
  () => route.query.site_id,
  (value) => {
    if (!siteSelectionInitialized.value) return
    const routeSiteId = stringQueryParam(value)
    if (routeSiteId === selectedSiteId.value) return
    setSelectedSite(routeSiteId, { syncRoute: false })
    void fetchTables()
  },
)

onMounted(async () => {
  await sitesStore.fetchSites()
  siteSelectionInitialized.value = true
  setSelectedSite(chooseInitialSiteId())
  await fetchTables()
})

onBeforeUnmount(() => {
  void closeBrowserConnection()
})
</script>

<template>
  <section class="space-y-5">
    <div class="flex flex-col gap-3 md:flex-row md:items-center md:justify-between">
      <div>
        <p class="text-xs font-medium uppercase tracking-[0.18em] text-muted-foreground">SurrealDB Explorer</p>
        <h1 class="mt-1 text-2xl font-semibold tracking-tight">数据浏览器</h1>
        <p class="mt-2 text-sm text-muted-foreground">
          使用浏览器 SurrealDB SDK 直连站点运行库，适合检查解析结果、模型关系，并在显式写入模式下执行维护语句。
        </p>
      </div>
      <button
        class="inline-flex h-9 items-center justify-center gap-2 rounded-md border border-input bg-background px-3 text-sm font-medium transition-colors hover:bg-accent disabled:cursor-not-allowed disabled:opacity-50"
        :disabled="!canUseDirectBrowser || loadingTables || loadingRecords"
        @click="refreshBrowser"
      >
        <RefreshCw class="h-4 w-4" :class="{ 'animate-spin': loadingTables || loadingRecords }" />
        刷新
      </button>
    </div>

    <div
      v-if="error"
      class="flex items-start gap-2 rounded-lg border border-destructive/30 bg-destructive/5 px-4 py-3 text-sm text-destructive"
    >
      <AlertTriangle class="mt-0.5 h-4 w-4 shrink-0" />
      <span>{{ error }}</span>
    </div>

    <div class="rounded-xl border border-border bg-card p-4 shadow-sm">
      <div class="flex flex-col gap-3 lg:flex-row lg:items-end lg:justify-between">
        <div class="min-w-0 flex-1">
          <label class="text-xs font-medium uppercase tracking-[0.18em] text-muted-foreground">
            当前站点数据库
          </label>
          <select
            :value="selectedSiteId"
            class="mt-2 h-10 w-full rounded-md border border-input bg-background px-3 text-sm outline-none transition-colors focus:border-primary lg:max-w-xl"
            :disabled="sitesStore.loading"
            @change="handleSiteSelect"
          >
            <option value="">请选择站点</option>
            <option v-for="site in sitesStore.sites" :key="site.site_id" :value="site.site_id">
              {{ siteDisplayName(site) }} · {{ site.status }} · runtime {{ site.runtime_db_mode }} · db:{{ site.db_port || '未分配' }}
            </option>
          </select>
        </div>
        <button
          class="inline-flex h-10 items-center justify-center gap-2 rounded-md border border-input bg-background px-3 text-sm font-medium transition-colors hover:bg-accent disabled:cursor-not-allowed disabled:opacity-50"
          :disabled="sitesStore.loading || loadingTables || loadingRecords"
          @click="refreshSites"
        >
          <RefreshCw class="h-4 w-4" :class="{ 'animate-spin': sitesStore.loading }" />
          更新站点
        </button>
      </div>
      <div class="mt-3 flex flex-col gap-2 text-sm text-muted-foreground md:flex-row md:items-center md:justify-between">
        <div class="min-w-0">
          <span class="font-medium text-foreground">{{ selectedSiteLabel }}</span>
          <span v-if="selectedSite" class="ml-2">
            project: {{ selectedSite.project_name }} · db: {{ selectedSite.db_port }} · runtime: {{ selectedSite.runtime_db_mode }}
          </span>
        </div>
        <span
          class="inline-flex w-fit items-center rounded-full px-2.5 py-1 text-xs font-medium"
          :class="canUseDirectBrowser ? 'bg-emerald-50 text-emerald-700' : 'bg-muted text-muted-foreground'"
        >
          {{ canUseDirectBrowser ? '可直连' : directBrowserBlockReason }}
        </span>
      </div>
      <div
        v-if="connection"
        class="mt-3 grid gap-2 rounded-lg border border-border bg-muted/30 p-3 text-xs text-muted-foreground md:grid-cols-3"
      >
        <div>
          <span class="font-medium text-foreground">连接</span>
          <div class="mt-1 font-mono">{{ connectionStatus }} · {{ connection.mode }} · {{ connection.credential.role }}</div>
        </div>
        <div>
          <span class="font-medium text-foreground">Namespace / Database</span>
          <div class="mt-1 font-mono">{{ connection.namespace }} / {{ connection.database }}</div>
        </div>
        <div>
          <span class="font-medium text-foreground">Endpoint</span>
          <div class="mt-1 truncate font-mono">{{ connection.endpoint }}</div>
        </div>
      </div>
    </div>

    <div class="rounded-xl border border-border bg-card p-4 shadow-sm">
      <div class="flex flex-col gap-3 lg:flex-row lg:items-start lg:justify-between">
        <div>
          <p class="text-xs font-medium uppercase tracking-[0.18em] text-muted-foreground">SurrealQL</p>
          <h2 class="mt-1 text-base font-semibold">SQL 查询编辑器</h2>
          <p class="mt-1 text-sm text-muted-foreground">
            默认使用 reader 凭据执行查询；检测到写操作时必须开启写入模式并二次确认。
          </p>
        </div>
        <label class="inline-flex items-center gap-2 rounded-lg border border-border bg-muted/30 px-3 py-2 text-sm">
          <input v-model="writeMode" type="checkbox" class="h-4 w-4 rounded border-input" />
          <span :class="writeMode ? 'font-semibold text-destructive' : 'text-muted-foreground'">写入模式</span>
        </label>
      </div>
      <textarea
        v-model="querySql"
        class="mt-4 min-h-32 w-full rounded-lg border border-input bg-background p-3 font-mono text-xs outline-none transition-colors placeholder:text-muted-foreground focus:border-primary"
        placeholder="SELECT * FROM table LIMIT 25;"
        :disabled="!canUseDirectBrowser || queryRunning"
      />
      <div class="mt-3 flex flex-col gap-3 md:flex-row md:items-center md:justify-between">
        <div class="text-xs" :class="queryNeedsWriteMode ? 'text-destructive' : 'text-muted-foreground'">
          {{ queryNeedsWriteMode ? '检测到写操作，需要写入模式和二次确认。' : '当前语句按只读查询处理。' }}
        </div>
        <button
          class="inline-flex h-9 items-center justify-center gap-2 rounded-md bg-primary px-4 text-sm font-medium text-primary-foreground transition-colors hover:bg-primary/90 disabled:cursor-not-allowed disabled:opacity-50"
          :disabled="!canUseDirectBrowser || queryRunning || (queryNeedsWriteMode && !writeMode)"
          @click="runSqlQuery"
        >
          <RefreshCw v-if="queryRunning" class="h-4 w-4 animate-spin" />
          执行 SurrealQL
        </button>
      </div>
      <div v-if="queryResult" class="mt-4 space-y-3">
        <div
          v-for="(response, index) in queryResult"
          :key="index"
          class="overflow-hidden rounded-lg border border-border"
        >
          <div
            class="flex items-center justify-between border-b border-border px-3 py-2 text-xs font-medium"
            :class="response.success ? 'bg-muted/40 text-foreground' : 'bg-destructive/5 text-destructive'"
          >
            <span>{{ queryResponseTitle(response, index) }}</span>
            <span>{{ response.success ? 'success' : 'failed' }}</span>
          </div>
          <pre class="max-h-80 overflow-auto bg-background p-3 font-mono text-xs text-foreground">{{ formatJson(response.success ? response.result : response.error) }}</pre>
        </div>
      </div>
    </div>

    <div class="grid min-h-[680px] overflow-hidden rounded-xl border border-border bg-card shadow-sm lg:grid-cols-[280px_minmax(0,1fr)]">
      <aside class="flex min-h-0 flex-col border-b border-border bg-muted/20 lg:border-b-0 lg:border-r">
        <div class="flex h-14 items-center justify-between border-b border-border px-4">
          <div class="flex items-center gap-2">
            <Table2 class="h-4 w-4 text-muted-foreground" />
            <span class="text-sm font-semibold">Tables</span>
            <span class="rounded-full bg-background px-2 py-0.5 text-xs text-muted-foreground">{{ tables.length }}</span>
          </div>
        </div>
        <div class="border-b border-border p-3">
          <label class="relative block">
            <Search class="pointer-events-none absolute left-3 top-2.5 h-4 w-4 text-muted-foreground" />
            <input
              v-model="tableSearch"
              class="h-9 w-full rounded-md border border-input bg-background pl-9 pr-3 text-sm outline-none transition-colors placeholder:text-muted-foreground focus:border-primary"
              placeholder="Search tables..."
              :disabled="!canUseDirectBrowser"
            />
          </label>
        </div>
        <div class="min-h-0 flex-1 overflow-auto p-2">
          <div v-if="!selectedSiteId" class="px-3 py-8 text-center text-sm text-muted-foreground">
            先选择一个站点。
          </div>
          <div v-else-if="!canUseDirectBrowser" class="px-3 py-8 text-center text-sm text-muted-foreground">
            {{ directBrowserBlockReason }}
          </div>
          <div v-else-if="loadingTables" class="px-3 py-8 text-center text-sm text-muted-foreground">
            正在读取表列表...
          </div>
          <div v-else-if="filteredTables.length === 0" class="px-3 py-8 text-center text-sm text-muted-foreground">
            未找到数据表
          </div>
          <button
            v-for="table in filteredTables"
            :key="table.name"
            class="mb-1 flex w-full items-center gap-2 rounded-lg px-3 py-2 text-left text-sm transition-colors"
            :class="activeTable === table.name
              ? 'bg-background text-foreground shadow-sm'
              : 'text-muted-foreground hover:bg-background/70 hover:text-foreground'"
            @click="selectTable(table.name)"
          >
            <Database class="h-4 w-4 shrink-0" />
            <span class="truncate font-medium">{{ table.name }}</span>
          </button>
        </div>
      </aside>

      <main class="flex min-w-0 flex-col">
        <div class="flex h-14 items-center justify-between gap-3 border-b border-border px-4">
          <div class="min-w-0">
            <div class="flex items-center gap-2">
              <h2 class="truncate text-sm font-semibold">
                {{ activeTable || 'Record Explorer' }}
              </h2>
              <span v-if="recordsResult" class="rounded-full bg-muted px-2 py-0.5 text-xs text-muted-foreground">
                {{ recordsResult.total.toLocaleString() }} rows
              </span>
            </div>
            <p class="mt-0.5 text-xs text-muted-foreground">
              {{ recordsResult ? `第 ${pageStart}-${pageEnd} 条，共 ${recordsResult.total.toLocaleString()} 条` : '选择左侧表开始浏览记录' }}
            </p>
          </div>
          <button
            class="inline-flex h-8 items-center justify-center gap-2 rounded-md border border-input bg-background px-3 text-xs font-medium transition-colors hover:bg-accent disabled:cursor-not-allowed disabled:opacity-50"
            :disabled="!canUseDirectBrowser || !activeTable || loadingRecords"
            @click="fetchRecords"
          >
            <RefreshCw class="h-3.5 w-3.5" :class="{ 'animate-spin': loadingRecords }" />
            记录刷新
          </button>
        </div>

        <div class="min-h-0 flex-1 overflow-auto">
          <div v-if="!selectedSiteId" class="flex h-full items-center justify-center text-sm text-muted-foreground">
            请选择一个站点开始浏览运行库。
          </div>
          <div v-else-if="!canUseDirectBrowser" class="flex h-full items-center justify-center px-6 text-center text-sm text-muted-foreground">
            {{ directBrowserBlockReason }}
          </div>
          <div v-else-if="!activeTable" class="flex h-full items-center justify-center text-sm text-muted-foreground">
            请选择一张数据表。
          </div>
          <div v-else-if="loadingRecords" class="flex h-full items-center justify-center text-sm text-muted-foreground">
            正在读取记录...
          </div>
          <div v-else-if="!recordsResult || recordsResult.records.length === 0" class="flex h-full items-center justify-center text-sm text-muted-foreground">
            当前页没有记录。
          </div>
          <table v-else class="w-full min-w-max text-sm">
            <thead class="sticky top-0 z-10 bg-muted/80 backdrop-blur">
              <tr class="border-b border-border">
                <th class="w-10 px-3 py-3">
                  <input type="checkbox" disabled class="h-4 w-4 rounded border-input" />
                </th>
                <th
                  v-for="column in recordsResult.columns"
                  :key="column"
                  class="cursor-pointer whitespace-nowrap px-4 py-3 text-left text-xs font-semibold uppercase tracking-wide text-foreground"
                  @click="toggleSort(column)"
                >
                  <span class="inline-flex items-center gap-1">
                    {{ column }}
                    <span v-if="sort === column" class="text-muted-foreground">{{ dir === 'asc' ? '↑' : '↓' }}</span>
                  </span>
                </th>
              </tr>
            </thead>
            <tbody>
              <tr
                v-for="(record, idx) in recordsResult.records"
                :key="`${activeTable}-${page}-${idx}`"
                class="border-b border-border/70 transition-colors hover:bg-muted/40"
              >
                <td class="px-3 py-2">
                  <input type="checkbox" disabled class="h-4 w-4 rounded border-input" />
                </td>
                <td
                  v-for="column in recordsResult.columns"
                  :key="column"
                  class="max-w-[320px] whitespace-nowrap px-4 py-2 font-mono text-xs"
                  :title="cellTitle(record[column])"
                >
                  <span
                    class="inline-flex max-w-full items-center gap-1 truncate"
                    :class="isRecordLink(record[column]) ? 'text-blue-600' : 'text-foreground'"
                  >
                    {{ formatCell(record[column]) }}
                    <ExternalLink v-if="isRecordLink(record[column])" class="h-3 w-3 shrink-0" />
                  </span>
                </td>
              </tr>
            </tbody>
          </table>
        </div>

        <div class="flex flex-col gap-3 border-t border-border px-4 py-3 sm:flex-row sm:items-center sm:justify-between">
          <div class="text-xs text-muted-foreground">
            {{ recordsResult ? `${pageStart}-${pageEnd} / ${recordsResult.total.toLocaleString()} rows` : '0 rows' }}
          </div>
          <div class="flex items-center gap-2">
            <button
              class="inline-flex h-8 w-8 items-center justify-center rounded-md border border-input bg-background transition-colors hover:bg-accent disabled:opacity-50"
              :disabled="!canUseDirectBrowser || page <= 1 || loadingRecords"
              @click="prevPage"
            >
              <ChevronLeft class="h-4 w-4" />
            </button>
            <span class="min-w-24 text-center text-xs text-muted-foreground">第 {{ page }} / {{ totalPages }} 页</span>
            <button
              class="inline-flex h-8 w-8 items-center justify-center rounded-md border border-input bg-background transition-colors hover:bg-accent disabled:opacity-50"
              :disabled="!canUseDirectBrowser || page >= totalPages || loadingRecords"
              @click="nextPage"
            >
              <ChevronRight class="h-4 w-4" />
            </button>
            <select
              v-model.number="perPage"
              class="h-8 rounded-md border border-input bg-background px-2 text-xs outline-none"
              :disabled="!canUseDirectBrowser"
              @change="page = 1"
            >
              <option :value="25">25 / 页</option>
              <option :value="50">50 / 页</option>
              <option :value="100">100 / 页</option>
            </select>
          </div>
        </div>
      </main>
    </div>
  </section>
</template>
