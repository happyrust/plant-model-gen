<script setup lang="ts">
import { computed, ref } from 'vue'
import { useRouter } from 'vue-router'
import { useSitesStore } from '@/stores/sites'
import type { ManagedProjectSite, ManagedSiteRiskLevel } from '@/types/site'
import { ArrowDown, ArrowUp, ArrowUpDown, Copy, Eye, ExternalLink, FolderPlus, Loader2, Pencil, Play, RefreshCw, RotateCcw, Square, Trash2 } from 'lucide-vue-next'
import {
  canDeleteSite,
  canEditSite,
  canParseSite,
  canRestartSite,
  canStartSite,
  canStopSite,
  parsePlanClass as getParsePlanClass,
  parseStatusClass as getParseStatusClass,
  statusClassMap,
  statusLabelMap,
} from './site-status'
import { buildViewerUrl } from '@/lib/viewer'
import SiteDeleteDialog from './SiteDeleteDialog.vue'

const props = defineProps<{
  sites: ManagedProjectSite[]
  loading: boolean
  selected?: string[]
}>()

// D6 / Sprint D · 修 G15：表头点击排序
//
// 默认按 updated_at desc（与后端 list_sites 顺序一致）；点击表头切换 column 与
// 升降序，再次点同一列在 asc <-> desc 之间翻转。纯前端排序，不动后端。
type SortColumn = 'project_name' | 'status' | 'web_port' | 'risk_level'
type SortDir = 'asc' | 'desc'

const RISK_RANK: Record<ManagedSiteRiskLevel, number> = {
  normal: 0,
  warning: 1,
  critical: 2,
}

const sortColumn = ref<SortColumn | null>(null)
const sortDir = ref<SortDir>('asc')

function toggleSort(column: SortColumn) {
  if (sortColumn.value === column) {
    sortDir.value = sortDir.value === 'asc' ? 'desc' : 'asc'
    return
  }
  sortColumn.value = column
  sortDir.value = 'asc'
}

function sortIcon(column: SortColumn) {
  if (sortColumn.value !== column) return ArrowUpDown
  return sortDir.value === 'asc' ? ArrowUp : ArrowDown
}

const sortedSites = computed(() => {
  if (sortColumn.value === null) return props.sites
  const column = sortColumn.value
  const factor = sortDir.value === 'asc' ? 1 : -1
  const arr = [...props.sites]
  arr.sort((a, b) => {
    let av: number | string
    let bv: number | string
    switch (column) {
      case 'project_name':
        av = a.project_name.toLowerCase()
        bv = b.project_name.toLowerCase()
        break
      case 'status':
        av = a.status
        bv = b.status
        break
      case 'web_port':
        av = a.web_port
        bv = b.web_port
        break
      case 'risk_level':
        av = RISK_RANK[a.risk_level] ?? 0
        bv = RISK_RANK[b.risk_level] ?? 0
        break
    }
    if (av < bv) return -1 * factor
    if (av > bv) return 1 * factor
    return 0
  })
  return arr
})

const emit = defineEmits<{
  'edit-site': [siteId: string]
  'clone-site': [siteId: string]
  'update-selection': [siteIds: string[]]
}>()

// D3 / Sprint D · 批量操作多选
//
// 选中的 site_id 集由父组件 (SitesView) 持有，本组件通过 v-model-like
// 协议（props.selected + emit('update-selection')）保持双向同步。
const selectedSet = computed(() => new Set(props.selected ?? []))

const allVisibleSelected = computed(() => {
  const visible = sortedSites.value
  return visible.length > 0 && visible.every((s) => selectedSet.value.has(s.site_id))
})

const someVisibleSelected = computed(() => {
  const visible = sortedSites.value
  return visible.some((s) => selectedSet.value.has(s.site_id))
})

function toggleRowSelection(siteId: string, checked: boolean) {
  const next = new Set(selectedSet.value)
  if (checked) next.add(siteId)
  else next.delete(siteId)
  emit('update-selection', [...next])
}

function toggleAllVisible(checked: boolean) {
  const visible = sortedSites.value.map((s) => s.site_id)
  if (checked) {
    const next = new Set(selectedSet.value)
    visible.forEach((id) => next.add(id))
    emit('update-selection', [...next])
  } else {
    const visibleSet = new Set(visible)
    emit('update-selection', [...selectedSet.value].filter((id) => !visibleSet.has(id)))
  }
}

const router = useRouter()
const sitesStore = useSitesStore()

function isPending(siteId: string) {
  return sitesStore.isSiteActionPending(siteId)
}
function pendingAction(siteId: string) {
  return sitesStore.getSiteAction(siteId)
}

const riskConfig: Record<ManagedSiteRiskLevel, { class: string; label: string }> = {
  normal: { class: 'bg-green-100 text-green-800 dark:bg-green-900 dark:text-green-200', label: '正常' },
  warning: { class: 'bg-amber-100 text-amber-800 dark:bg-amber-900 dark:text-amber-200', label: '警告' },
  critical: { class: 'bg-red-100 text-red-800 dark:bg-red-900 dark:text-red-200', label: '严重' },
}

const canStart = canStartSite
const canStop = canStopSite
const canParse = canParseSite
const canRestart = canRestartSite
const canDelete = canDeleteSite
const canEdit = canEditSite

// D2 / Sprint D · 修 G9：用 hi-fi 弹框替代 window.confirm
const deleteTarget = ref<ManagedProjectSite | null>(null)
const deletePending = ref(false)

function confirmDelete(site: ManagedProjectSite) {
  deleteTarget.value = site
}

function cancelDelete() {
  if (deletePending.value) return
  deleteTarget.value = null
}

async function executeDelete() {
  const site = deleteTarget.value
  if (!site || deletePending.value) return
  deletePending.value = true
  try {
    await sitesStore.deleteSite(site.site_id)
    deleteTarget.value = null
  } catch {
    // 错误已写入 store，弹框保持打开方便用户重试或关闭
  } finally {
    deletePending.value = false
  }
}
function riskSummary(site: ManagedProjectSite) {
  return site.risk_reasons[0] ?? '当前无明显风险'
}
function openDetail(siteId: string) {
  router.push({ path: '/sites/' + siteId })
}
function openViewer(site: ManagedProjectSite) {
  const url = buildViewerUrl(site)
  if (url) window.open(url, '_blank')
}

async function handleStart(siteId: string) {
  try {
    await sitesStore.startSite(siteId)
  } catch {
    // 错误已写入 store
  }
}

async function handleStop(siteId: string) {
  try {
    await sitesStore.stopSite(siteId)
  } catch {
    // 错误已写入 store
  }
}

async function handleRestart(siteId: string) {
  try {
    await sitesStore.restartSite(siteId)
  } catch {
    // 错误已写入 store
  }
}

async function handleParse(siteId: string) {
  try {
    await sitesStore.parseSite(siteId)
  } catch {
    // 错误已写入 store
  }
}

</script>

<template>
  <div class="rounded-lg border border-border">
    <div v-if="loading && props.sites.length === 0" class="p-10">
      <div class="space-y-3">
        <div v-for="i in 3" :key="i" class="flex gap-4 animate-pulse">
          <div class="h-5 w-40 rounded bg-muted"></div>
          <div class="h-5 w-20 rounded bg-muted"></div>
          <div class="h-5 w-16 rounded bg-muted"></div>
          <div class="h-5 w-24 rounded bg-muted"></div>
          <div class="flex-1"></div>
          <div class="h-5 w-28 rounded bg-muted"></div>
        </div>
      </div>
    </div>
    <div v-else-if="props.sites.length === 0" class="p-10 text-center">
      <FolderPlus class="mx-auto h-10 w-10 text-muted-foreground/40" />
      <p class="mt-3 text-sm font-medium text-muted-foreground">还没有站点</p>
      <p class="mt-1 text-xs text-muted-foreground">点击右上角「新建站点」开始创建第一个站点</p>
    </div>
    <table v-else class="w-full text-sm">
      <thead>
        <tr class="border-b border-border bg-muted/50">
          <th class="w-10 px-3 py-3 text-left font-medium text-muted-foreground">
            <input
              type="checkbox"
              class="h-4 w-4 cursor-pointer rounded border-input"
              :checked="allVisibleSelected"
              :indeterminate="someVisibleSelected && !allVisibleSelected"
              :title="allVisibleSelected ? '取消全选' : '全选当前可见站点'"
              @change="toggleAllVisible(($event.target as HTMLInputElement).checked)"
              @click.stop
            />
          </th>
          <th class="px-4 py-3 text-left font-medium text-muted-foreground">
            <button type="button" class="inline-flex items-center gap-1 hover:text-foreground transition-colors" @click="toggleSort('project_name')">
              项目名称 <component :is="sortIcon('project_name')" class="h-3.5 w-3.5" />
            </button>
          </th>
          <th class="px-4 py-3 text-left font-medium text-muted-foreground">
            <button type="button" class="inline-flex items-center gap-1 hover:text-foreground transition-colors" @click="toggleSort('status')">
              状态 <component :is="sortIcon('status')" class="h-3.5 w-3.5" />
            </button>
          </th>
          <th class="px-4 py-3 text-left font-medium text-muted-foreground">
            <button type="button" class="inline-flex items-center gap-1 hover:text-foreground transition-colors" @click="toggleSort('web_port')">
              端口 <component :is="sortIcon('web_port')" class="h-3.5 w-3.5" />
            </button>
          </th>
          <th class="px-4 py-3 text-left font-medium text-muted-foreground">
            <button type="button" class="inline-flex items-center gap-1 hover:text-foreground transition-colors" @click="toggleSort('risk_level')">
              风险 <component :is="sortIcon('risk_level')" class="h-3.5 w-3.5" />
            </button>
          </th>
          <th class="px-4 py-3 text-right font-medium text-muted-foreground">操作</th>
        </tr>
      </thead>
      <tbody>
        <tr
          v-for="site in sortedSites"
          :key="site.site_id"
          class="border-b border-border last:border-0 hover:bg-muted/30 transition-colors cursor-pointer"
          :class="selectedSet.has(site.site_id) ? 'bg-accent/20' : ''"
          @click="openDetail(site.site_id)"
        >
          <td class="w-10 px-3 py-3 align-top" @click.stop>
            <input
              type="checkbox"
              class="h-4 w-4 cursor-pointer rounded border-input"
              :checked="selectedSet.has(site.site_id)"
              @change="toggleRowSelection(site.site_id, ($event.target as HTMLInputElement).checked)"
            />
          </td>
          <td class="px-4 py-3 align-top">
            <div class="font-medium">{{ site.project_name }}</div>
            <div class="text-xs text-muted-foreground">{{ site.site_id }}</div>
            <a
              v-if="site.entry_url && site.status === 'Running'"
              :href="site.public_entry_url || site.entry_url"
              target="_blank"
              rel="noreferrer"
              class="mt-1 inline-flex items-center gap-1 text-xs text-primary hover:underline"
              @click.stop
            >
              {{ site.public_entry_url || site.entry_url }}
              <ExternalLink class="h-3 w-3" />
            </a>
            <div v-if="site.last_error" class="mt-1 max-w-[280px] truncate text-xs text-destructive" :title="site.last_error">
              {{ site.last_error }}
            </div>
          </td>
          <td class="px-4 py-3 align-top">
            <div class="flex flex-wrap items-center gap-1.5">
              <span class="inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium" :class="statusClassMap[site.status]">
                {{ statusLabelMap[site.status] ?? site.status }}
              </span>
              <span class="inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium" :class="getParseStatusClass(site.parse_status)">
                {{ site.parse_status }}
              </span>
            </div>
            <div v-if="site.parse_plan?.label" class="mt-2 space-y-1">
              <span
                class="inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium"
                :class="getParsePlanClass(site.parse_plan)"
              >
                {{ site.parse_plan.label }}
              </span>
              <div class="max-w-[260px] truncate text-xs text-muted-foreground" :title="site.parse_plan.detail">
                {{ site.parse_plan.detail }}
              </div>
            </div>
          </td>
          <td class="px-4 py-3 align-top">
            <div class="font-mono text-xs text-muted-foreground">
              <span title="DB 端口">D:{{ site.db_port }}</span>
              <span class="mx-1">·</span>
              <span title="Web 端口">W:{{ site.web_port }}</span>
            </div>
          </td>
          <td class="px-4 py-3 align-top">
            <span
              class="inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium"
              :class="riskConfig[site.risk_level]?.class"
              :title="site.risk_reasons.join('；') || '当前无明显风险'"
            >
              {{ riskConfig[site.risk_level]?.label ?? site.risk_level }}
            </span>
            <div v-if="riskSummary(site) !== '当前无明显风险'" class="mt-1 max-w-[200px] truncate text-xs text-muted-foreground" :title="site.risk_reasons.join('；')">
              {{ riskSummary(site) }}
            </div>
          </td>
          <td class="px-4 py-3 text-right align-top" @click.stop>
            <div v-if="isPending(site.site_id)" class="flex items-center justify-end gap-2">
              <Loader2 class="h-4 w-4 animate-spin text-muted-foreground" />
              <span class="text-xs text-muted-foreground">
                {{ pendingAction(site.site_id) === 'start' ? '启动中' : pendingAction(site.site_id) === 'stop' ? '停止中' : pendingAction(site.site_id) === 'restart' ? '重启中' : pendingAction(site.site_id) === 'parse' ? '解析中' : '处理中' }}
              </span>
            </div>
            <div v-else class="flex items-center justify-end gap-1">
              <button
                v-if="canStart(site)"
                @click="handleStart(site.site_id)"
                class="inline-flex h-7 w-7 items-center justify-center rounded-md hover:bg-accent transition-colors"
                title="启动"
              >
                <Play class="h-3.5 w-3.5 text-green-600" />
              </button>
              <button
                v-if="canStop(site)"
                @click="handleStop(site.site_id)"
                class="inline-flex h-7 w-7 items-center justify-center rounded-md hover:bg-accent transition-colors"
                title="停止"
              >
                <Square class="h-3.5 w-3.5 text-amber-600" />
              </button>
              <button
                v-if="canRestart(site)"
                @click="handleRestart(site.site_id)"
                class="inline-flex h-7 w-7 items-center justify-center rounded-md hover:bg-accent transition-colors"
                title="重启"
              >
                <RotateCcw class="h-3.5 w-3.5 text-blue-600" />
              </button>
              <button
                v-if="canParse(site)"
                @click="handleParse(site.site_id)"
                class="inline-flex h-7 w-7 items-center justify-center rounded-md hover:bg-accent transition-colors"
                title="解析"
              >
                <RefreshCw class="h-3.5 w-3.5" />
              </button>
              <button
                v-if="site.status === 'Running' && buildViewerUrl(site)"
                @click="openViewer(site)"
                class="inline-flex h-7 w-7 items-center justify-center rounded-md hover:bg-accent transition-colors"
                :title="`打开 Viewer (${site.associated_project || site.project_name})`"
              >
                <Eye class="h-3.5 w-3.5 text-primary" />
              </button>
              <button
                @click="openDetail(site.site_id)"
                class="inline-flex h-7 w-7 items-center justify-center rounded-md hover:bg-accent transition-colors"
                title="详情"
              >
                <ExternalLink class="h-3.5 w-3.5" />
              </button>
              <button
                v-if="canEdit(site)"
                @click="emit('edit-site', site.site_id)"
                class="inline-flex h-7 w-7 items-center justify-center rounded-md hover:bg-accent transition-colors"
                title="编辑配置"
              >
                <Pencil class="h-3.5 w-3.5" />
              </button>
              <button
                @click="emit('clone-site', site.site_id)"
                class="inline-flex h-7 w-7 items-center justify-center rounded-md hover:bg-accent transition-colors"
                title="克隆站点（端口自动 +1，需重填凭据）"
              >
                <Copy class="h-3.5 w-3.5" />
              </button>
              <button
                v-if="canDelete(site)"
                @click="confirmDelete(site)"
                class="inline-flex h-7 w-7 items-center justify-center rounded-md hover:bg-accent transition-colors"
                title="删除"
              >
                <Trash2 class="h-3.5 w-3.5 text-destructive" />
              </button>
            </div>
          </td>
        </tr>
      </tbody>
    </table>
    <SiteDeleteDialog
      :open="deleteTarget !== null"
      :site="deleteTarget"
      :pending="deletePending"
      @cancel="cancelDelete"
      @confirm="executeDelete"
    />
  </div>
</template>
