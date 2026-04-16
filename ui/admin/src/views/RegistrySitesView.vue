<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'
import {
  ArrowLeft,
  ArrowRight,
  ClipboardCopy,
  ExternalLink,
  FileCog,
  FileDown,
  HeartPulse,
  Pencil,
  Plus,
  RefreshCw,
  Search,
  Trash2,
} from 'lucide-vue-next'

import RegistrySiteDrawer from '@/components/registry/RegistrySiteDrawer.vue'
import { findLinkedLocalSite, getRegistryStatusLabel } from '@/lib/registry'
import { usePolling } from '@/composables/usePolling'
import { useRegistryStore } from '@/stores/registry'
import { useSitesStore } from '@/stores/sites'
import type { RegistrySite } from '@/types/registry'

type FeedbackTone = 'success' | 'error' | 'info'

const router = useRouter()
const registryStore = useRegistryStore()
const sitesStore = useSitesStore()

const searchQuery = ref('')
const statusFilter = ref('')
const regionFilter = ref('')
const ownerFilter = ref('')
const envFilter = ref('')
const projectNameFilter = ref('')
const sortFilter = ref('')
const drawerOpen = ref(false)
const editingSiteId = ref<string | null>(null)
const feedback = ref<{ tone: FeedbackTone; message: string } | null>(null)

const importDialogOpen = ref(false)
const importPath = ref('db_options/DbOption.toml')
const importLoading = ref(false)
const importError = ref('')

const deleteTarget = ref<RegistrySite | null>(null)
const deleteLoading = ref(false)

const { start: startPolling } = usePolling(async () => {
  await refreshAll({ silent: true })
}, 30000)

const paginationSummary = computed(() => {
  if (registryStore.total === 0) {
    return '暂无数据'
  }

  const start = (registryStore.page - 1) * registryStore.perPage + 1
  const end = Math.min(registryStore.total, start + registryStore.sites.length - 1)
  return `第 ${start}-${end} 条，共 ${registryStore.total} 条`
})

const localSiteMap = computed(() => {
  const result = new Map<string, ReturnType<typeof findLinkedLocalSite>>()
  for (const site of registryStore.sites) {
    result.set(site.site_id, findLinkedLocalSite(site, sitesStore.sites))
  }
  return result
})

const statusBadgeClass = (status: string) => {
  const styles: Record<string, string> = {
    Running: 'bg-green-100 text-green-800 dark:bg-green-900 dark:text-green-200',
    Failed: 'bg-red-100 text-red-800 dark:bg-red-900 dark:text-red-200',
    Offline: 'bg-amber-100 text-amber-800 dark:bg-amber-900 dark:text-amber-200',
    Deploying: 'bg-blue-100 text-blue-800 dark:bg-blue-900 dark:text-blue-200',
    Configuring: 'bg-muted text-muted-foreground',
    Stopped: 'bg-muted text-muted-foreground',
  }
  return styles[status] ?? 'bg-muted text-muted-foreground'
}

function setFeedback(tone: FeedbackTone, message: string) {
  feedback.value = { tone, message }
}

async function refreshAll(options?: { silent?: boolean }) {
  if (!options?.silent) {
    feedback.value = null
  }

  const currentQuery = registryStore.query
  searchQuery.value = currentQuery.q ?? ''
  statusFilter.value = currentQuery.status ?? ''
  regionFilter.value = currentQuery.region ?? ''

  await registryStore.fetchSites()
  try {
    await sitesStore.fetchSites()
  } catch {
    // 本机编排页本身已有独立错误提示，这里不阻塞注册表页面
  }
}

async function applyFilters() {
  feedback.value = null
  try {
    await registryStore.fetchSites({
      q: searchQuery.value.trim() || undefined,
      status: statusFilter.value || undefined,
      region: regionFilter.value.trim() || undefined,
      owner: ownerFilter.value.trim() || undefined,
      env: envFilter.value.trim() || undefined,
      project_name: projectNameFilter.value.trim() || undefined,
      sort: sortFilter.value || undefined,
      page: 1,
    })
  } catch (err: unknown) {
    setFeedback('error', err instanceof Error ? err.message : '筛选注册表站点失败')
  }
}

function openCreateDrawer() {
  editingSiteId.value = null
  drawerOpen.value = true
}

function openEditDrawer(siteId: string) {
  editingSiteId.value = siteId
  drawerOpen.value = true
}

async function handleDrawerSaved() {
  drawerOpen.value = false
  editingSiteId.value = null
  await refreshAll({ silent: true })
  setFeedback('success', '注册表站点已保存')
}

function openImportDialog() {
  importPath.value = 'db_options/DbOption.toml'
  importError.value = ''
  importDialogOpen.value = true
}

async function confirmImport() {
  importLoading.value = true
  importError.value = ''
  try {
    await registryStore.importSite({ path: importPath.value.trim() || 'db_options/DbOption.toml' })
    importDialogOpen.value = false
    await refreshAll({ silent: true })
    setFeedback('success', '已从 DbOption 导入注册表站点')
  } catch (err: unknown) {
    importError.value = err instanceof Error ? err.message : '导入注册表站点失败'
  } finally {
    importLoading.value = false
  }
}

async function handleHealthcheck(site: RegistrySite) {
  try {
    const result = await registryStore.healthcheckSite(site.site_id)
    setFeedback(
      result.healthy ? 'success' : 'error',
      result.healthy
        ? `${site.name} 健康检查通过`
        : `${site.name} 健康检查失败`,
    )
  } catch (err: unknown) {
    setFeedback('error', err instanceof Error ? err.message : '执行健康检查失败')
  }
}

function handleCreateTask(site: RegistrySite) {
  router.push({ path: '/tasks/new', query: { site_id: site.site_id, site_label: site.name } })
}

async function handleExport(site: RegistrySite) {
  try {
    const result = await registryStore.exportConfig(site.site_id)
    const blob = new Blob(
      [JSON.stringify(result.config, null, 2)],
      { type: 'application/json;charset=utf-8' },
    )
    const url = URL.createObjectURL(blob)
    const link = document.createElement('a')
    link.href = url
    link.download = `${result.name || site.site_id}-config.json`
    link.click()
    URL.revokeObjectURL(url)
    setFeedback('success', `${site.name} 配置已导出`)
  } catch (err: unknown) {
    setFeedback('error', err instanceof Error ? err.message : '导出配置失败')
  }
}

function openDeleteConfirm(site: RegistrySite) {
  deleteTarget.value = site
}

async function confirmDelete() {
  if (!deleteTarget.value) return
  deleteLoading.value = true
  try {
    const name = deleteTarget.value.name
    await registryStore.deleteSite(deleteTarget.value.site_id)
    deleteTarget.value = null
    setFeedback('success', `${name} 已删除`)
  } catch (err: unknown) {
    setFeedback('error', err instanceof Error ? err.message : '删除注册表站点失败')
    deleteTarget.value = null
  } finally {
    deleteLoading.value = false
  }
}

function copyToClipboard(text: string) {
  navigator.clipboard.writeText(text)
  setFeedback('info', '已复制到剪贴板')
}

async function changePage(nextPage: number) {
  if (nextPage < 1 || nextPage > registryStore.pages || nextPage === registryStore.page) {
    return
  }

  try {
    await registryStore.fetchSites({ page: nextPage })
  } catch (err: unknown) {
    setFeedback('error', err instanceof Error ? err.message : '切换分页失败')
  }
}

function openLocalSite(siteId: string) {
  router.push(`/sites/${siteId}`)
}

onMounted(async () => {
  searchQuery.value = registryStore.query.q ?? ''
  statusFilter.value = registryStore.query.status ?? ''
  regionFilter.value = registryStore.query.region ?? ''
  try {
    await refreshAll({ silent: true })
  } catch (err: unknown) {
    setFeedback('error', err instanceof Error ? err.message : '加载注册表站点失败')
  }
  startPolling()
})
</script>

<template>
  <div class="space-y-6">
    <div class="flex flex-wrap items-start justify-between gap-4">
      <div>
        <h2 class="text-2xl font-semibold tracking-tight">中心注册表</h2>
        <p class="text-sm text-muted-foreground">
          统一维护中心注册表记录，并和本机编排页面建立联通关系
        </p>
      </div>
      <div class="flex flex-wrap items-center gap-2">
        <button
          class="inline-flex h-9 items-center gap-2 rounded-md border border-input px-4 text-sm font-medium hover:bg-accent transition-colors"
          @click="refreshAll()"
        >
          <RefreshCw class="h-4 w-4" />
          刷新
        </button>
        <button
          class="inline-flex h-9 items-center gap-2 rounded-md border border-input px-4 text-sm font-medium hover:bg-accent transition-colors"
          @click="openImportDialog"
        >
          <FileCog class="h-4 w-4" />
          从 DbOption 导入
        </button>
        <button
          class="inline-flex h-9 items-center gap-2 rounded-md bg-primary px-4 text-sm font-medium text-primary-foreground shadow hover:bg-primary/90 transition-colors"
          @click="openCreateDrawer"
        >
          <Plus class="h-4 w-4" />
          新建注册表站点
        </button>
      </div>
    </div>

    <div
      v-if="feedback"
      class="rounded-lg border px-4 py-3 text-sm"
      :class="feedback.tone === 'success'
        ? 'border-green-200 bg-green-50 text-green-700'
        : feedback.tone === 'error'
          ? 'border-red-200 bg-red-50 text-red-700'
          : 'border-blue-200 bg-blue-50 text-blue-700'"
    >
      {{ feedback.message }}
    </div>

    <div class="grid gap-4 md:grid-cols-4">
      <div class="rounded-lg border border-border bg-card p-4">
        <p class="text-sm text-muted-foreground">总站点</p>
        <p class="mt-2 text-2xl font-semibold">{{ registryStore.stats.total }}</p>
      </div>
      <div class="rounded-lg border border-border bg-card p-4">
        <p class="text-sm text-muted-foreground">运行中</p>
        <p class="mt-2 text-2xl font-semibold text-green-600">{{ registryStore.stats.running }}</p>
      </div>
      <div class="rounded-lg border border-border bg-card p-4">
        <p class="text-sm text-muted-foreground">失败</p>
        <p class="mt-2 text-2xl font-semibold text-destructive">{{ registryStore.stats.failed }}</p>
      </div>
      <div class="rounded-lg border border-border bg-card p-4">
        <p class="text-sm text-muted-foreground">离线</p>
        <p class="mt-2 text-2xl font-semibold text-amber-600">{{ registryStore.stats.offline }}</p>
      </div>
    </div>

    <div class="flex flex-wrap items-center gap-3 rounded-lg border border-border bg-card p-4">
      <div class="relative min-w-[240px] flex-1">
        <Search class="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
        <input
          v-model="searchQuery"
          type="text"
          placeholder="搜索站点名 / 项目名 / 地址"
          class="flex h-10 w-full rounded-md border border-input bg-transparent pl-9 pr-3 text-sm shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
          @keydown.enter.prevent="applyFilters"
        />
      </div>
      <select
        v-model="statusFilter"
        class="flex h-10 rounded-md border border-input bg-transparent px-3 text-sm shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
      >
        <option value="">所有状态</option>
        <option value="Configuring">配置中</option>
        <option value="Deploying">部署中</option>
        <option value="Running">运行中</option>
        <option value="Failed">失败</option>
        <option value="Stopped">已停止</option>
        <option value="Offline">离线</option>
      </select>
      <input
        v-model="regionFilter"
        type="text"
        placeholder="区域"
        class="flex h-10 min-w-[140px] rounded-md border border-input bg-transparent px-3 text-sm shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
        @keydown.enter.prevent="applyFilters"
      />
      <button
        class="inline-flex h-10 items-center rounded-md border border-input px-4 text-sm font-medium hover:bg-accent transition-colors"
        @click="applyFilters"
      >
        应用筛选
      </button>
    </div>

    <div class="rounded-lg border border-border">
      <div
        v-if="registryStore.loading && !registryStore.sites.length"
        class="p-10 text-center text-sm text-muted-foreground"
      >
        正在加载注册表站点...
      </div>
      <div
        v-else-if="!registryStore.sites.length"
        class="p-10 text-center text-sm text-muted-foreground"
      >
        暂无注册表站点
      </div>
      <table v-else class="w-full text-sm">
        <thead>
          <tr class="border-b border-border bg-muted/50">
            <th class="px-4 py-3 text-left font-medium text-muted-foreground">站点</th>
            <th class="px-4 py-3 text-left font-medium text-muted-foreground">项目</th>
            <th class="px-4 py-3 text-left font-medium text-muted-foreground">状态</th>
            <th class="px-4 py-3 text-left font-medium text-muted-foreground">后端地址</th>
            <th class="px-4 py-3 text-left font-medium text-muted-foreground">本机编排</th>
            <th class="px-4 py-3 text-left font-medium text-muted-foreground">更新时间</th>
            <th class="px-4 py-3 text-right font-medium text-muted-foreground">操作</th>
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="site in registryStore.sites"
            :key="site.site_id"
            class="border-b border-border align-top last:border-0 hover:bg-muted/20 transition-colors"
          >
            <td class="px-4 py-3">
              <div class="font-medium">{{ site.name }}</div>
              <div class="text-xs text-muted-foreground">{{ site.site_id }}</div>
              <div class="mt-1 text-xs text-muted-foreground">
                {{ site.region || site.env || '未设置区域' }}
              </div>
            </td>
            <td class="px-4 py-3">
              <div>{{ site.project_name || '-' }}</div>
              <div class="text-xs text-muted-foreground">{{ site.project_path || '-' }}</div>
            </td>
            <td class="px-4 py-3">
              <span
                class="inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium"
                :class="statusBadgeClass(site.status)"
              >
                {{ getRegistryStatusLabel(site.status) }}
              </span>
              <div class="mt-1 text-xs text-muted-foreground">
                最近心跳：{{ site.last_seen_at || '-' }}
              </div>
            </td>
            <td class="px-4 py-3">
              <div v-if="site.backend_url" class="flex items-center gap-1">
                <a
                  :href="site.backend_url"
                  target="_blank"
                  rel="noreferrer"
                  class="inline-flex items-center gap-1 text-primary hover:underline"
                >
                  <span>{{ site.backend_url }}</span>
                  <ExternalLink class="h-3.5 w-3.5" />
                </a>
                <button
                  class="inline-flex h-6 w-6 items-center justify-center rounded hover:bg-accent transition-colors"
                  title="复制地址"
                  @click.stop="copyToClipboard(site.backend_url)"
                >
                  <ClipboardCopy class="h-3 w-3 text-muted-foreground" />
                </button>
              </div>
              <div v-else class="text-muted-foreground">-</div>
            </td>
            <td class="px-4 py-3">
              <button
                v-if="localSiteMap.get(site.site_id)"
                class="inline-flex items-center gap-1 text-primary hover:underline"
                @click="openLocalSite(localSiteMap.get(site.site_id)?.site_id || '')"
              >
                跳到本机站点
                <ExternalLink class="h-3.5 w-3.5" />
              </button>
              <span v-else class="text-muted-foreground">未匹配</span>
            </td>
            <td class="px-4 py-3 text-muted-foreground">
              {{ site.updated_at || site.created_at || '-' }}
            </td>
            <td class="px-4 py-3 text-right">
              <div class="flex items-center justify-end gap-1">
                <button
                  class="inline-flex h-8 w-8 items-center justify-center rounded-md hover:bg-accent transition-colors"
                  title="健康检查"
                  @click="handleHealthcheck(site)"
                >
                  <HeartPulse class="h-4 w-4 text-emerald-600" />
                </button>
                <button
                  class="inline-flex h-8 w-8 items-center justify-center rounded-md hover:bg-accent transition-colors"
                  title="导出配置"
                  @click="handleExport(site)"
                >
                  <FileDown class="h-4 w-4 text-blue-600" />
                </button>
                <button
                  class="inline-flex h-8 w-8 items-center justify-center rounded-md hover:bg-accent transition-colors"
                  title="创建任务"
                  @click="handleCreateTask(site)"
                >
                  <FileCog class="h-4 w-4" />
                </button>
                <button
                  class="inline-flex h-8 w-8 items-center justify-center rounded-md hover:bg-accent transition-colors"
                  title="编辑"
                  @click="openEditDrawer(site.site_id)"
                >
                  <Pencil class="h-4 w-4" />
                </button>
                <button
                  class="inline-flex h-8 w-8 items-center justify-center rounded-md hover:bg-accent transition-colors"
                  title="删除"
                  @click="openDeleteConfirm(site)"
                >
                  <Trash2 class="h-4 w-4 text-destructive" />
                </button>
              </div>
            </td>
          </tr>
        </tbody>
      </table>
    </div>

    <div class="flex flex-wrap items-center justify-between gap-3">
      <p class="text-sm text-muted-foreground">{{ paginationSummary }}</p>
      <div class="flex items-center gap-2">
        <button
          class="inline-flex h-9 items-center gap-1 rounded-md border border-input px-3 text-sm font-medium hover:bg-accent transition-colors disabled:opacity-50"
          :disabled="registryStore.page <= 1"
          @click="changePage(registryStore.page - 1)"
        >
          <ArrowLeft class="h-4 w-4" />
          上一页
        </button>
        <span class="text-sm text-muted-foreground">
          第 {{ registryStore.page }} / {{ registryStore.pages }} 页
        </span>
        <button
          class="inline-flex h-9 items-center gap-1 rounded-md border border-input px-3 text-sm font-medium hover:bg-accent transition-colors disabled:opacity-50"
          :disabled="registryStore.page >= registryStore.pages"
          @click="changePage(registryStore.page + 1)"
        >
          下一页
          <ArrowRight class="h-4 w-4" />
        </button>
      </div>
    </div>

    <RegistrySiteDrawer
      :open="drawerOpen"
      :site-id="editingSiteId"
      @close="drawerOpen = false"
      @saved="handleDrawerSaved"
    />

    <Teleport to="body">
      <div v-if="importDialogOpen" class="fixed inset-0 z-50 flex items-center justify-center">
        <div class="absolute inset-0 bg-black/50" @click="importDialogOpen = false" />
        <div class="relative w-full max-w-md rounded-lg border border-border bg-background p-6 shadow-xl">
          <h3 class="text-lg font-semibold mb-4">从 DbOption 导入</h3>
          <div class="space-y-3">
            <input
              v-model="importPath"
              type="text"
              placeholder="db_options/DbOption.toml"
              class="flex h-10 w-full rounded-md border border-input bg-transparent px-3 text-sm shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
            />
            <div v-if="importError" class="rounded-md border border-destructive/50 bg-destructive/10 p-3 text-sm text-destructive">
              {{ importError }}
            </div>
          </div>
          <div class="mt-4 flex justify-end gap-3">
            <button
              class="inline-flex h-9 items-center rounded-md border border-input bg-transparent px-4 text-sm font-medium hover:bg-accent transition-colors"
              @click="importDialogOpen = false"
            >取消</button>
            <button
              class="inline-flex h-9 items-center rounded-md bg-primary px-4 text-sm font-medium text-primary-foreground shadow hover:bg-primary/90 transition-colors disabled:opacity-50"
              :disabled="importLoading"
              @click="confirmImport"
            >{{ importLoading ? '导入中...' : '导入' }}</button>
          </div>
        </div>
      </div>
    </Teleport>

    <Teleport to="body">
      <div v-if="deleteTarget" class="fixed inset-0 z-50 flex items-center justify-center">
        <div class="absolute inset-0 bg-black/50" @click="deleteTarget = null" />
        <div class="relative w-full max-w-sm rounded-lg border border-border bg-background p-6 shadow-xl">
          <h3 class="text-lg font-semibold mb-2">确认删除</h3>
          <p class="text-sm text-muted-foreground">确认删除注册表站点「{{ deleteTarget.name }}」吗？此操作不可撤销。</p>
          <div class="mt-4 flex justify-end gap-3">
            <button
              class="inline-flex h-9 items-center rounded-md border border-input bg-transparent px-4 text-sm font-medium hover:bg-accent transition-colors"
              @click="deleteTarget = null"
            >取消</button>
            <button
              class="inline-flex h-9 items-center rounded-md bg-destructive px-4 text-sm font-medium text-destructive-foreground shadow hover:bg-destructive/90 transition-colors disabled:opacity-50"
              :disabled="deleteLoading"
              @click="confirmDelete"
            >{{ deleteLoading ? '删除中...' : '删除' }}</button>
          </div>
        </div>
      </div>
    </Teleport>
  </div>
</template>
