<script setup lang="ts">
import { useRouter } from 'vue-router'
import { useSitesStore } from '@/stores/sites'
import type { ManagedProjectSite, ManagedSiteRiskLevel, ManagedSiteStatus } from '@/types/site'
import { Eye, ExternalLink, Play, RefreshCw, Square, Trash2 } from 'lucide-vue-next'

const props = defineProps<{ sites: ManagedProjectSite[]; loading: boolean }>()

const router = useRouter()
const sitesStore = useSitesStore()

const statusConfig: Record<ManagedSiteStatus, { class: string; label: string }> = {
  Draft: { class: 'bg-muted text-muted-foreground', label: '草稿' },
  Parsed: { class: 'bg-blue-100 text-blue-800 dark:bg-blue-900 dark:text-blue-200', label: '已解析' },
  Starting: { class: 'bg-amber-100 text-amber-800 dark:bg-amber-900 dark:text-amber-200', label: '启动中' },
  Running: { class: 'bg-green-100 text-green-800 dark:bg-green-900 dark:text-green-200', label: '运行中' },
  Stopping: { class: 'bg-amber-100 text-amber-800 dark:bg-amber-900 dark:text-amber-200', label: '停止中' },
  Stopped: { class: 'bg-muted text-muted-foreground', label: '已停止' },
  Failed: { class: 'bg-red-100 text-red-800 dark:bg-red-900 dark:text-red-200', label: '失败' },
}

const riskConfig: Record<ManagedSiteRiskLevel, { class: string; label: string }> = {
  normal: { class: 'bg-green-100 text-green-800 dark:bg-green-900 dark:text-green-200', label: '正常' },
  warning: { class: 'bg-amber-100 text-amber-800 dark:bg-amber-900 dark:text-amber-200', label: '警告' },
  critical: { class: 'bg-red-100 text-red-800 dark:bg-red-900 dark:text-red-200', label: '严重' },
}

function canStart(site: ManagedProjectSite) {
  return ['Stopped', 'Parsed', 'Failed', 'Draft'].includes(site.status)
}
function canStop(site: ManagedProjectSite) {
  return site.status === 'Running'
}
function canParse(site: ManagedProjectSite) {
  if (site.parse_status === 'Running') return false
  return ['Starting', 'Stopping'].includes(site.status) === false
}
function canDelete(site: ManagedProjectSite) {
  return ['Running', 'Starting', 'Stopping'].includes(site.status) === false
}
function riskSummary(site: ManagedProjectSite) {
  return site.risk_reasons[0] ?? '当前无明显风险'
}
function parseStatusClass(site: ManagedProjectSite) {
  if (site.parse_status === 'Parsed') return 'bg-green-100 text-green-800 dark:bg-green-900 dark:text-green-200'
  if (site.parse_status === 'Running') return 'bg-blue-100 text-blue-800 dark:bg-blue-900 dark:text-blue-200'
  if (site.parse_status === 'Failed') return 'bg-red-100 text-red-800 dark:bg-red-900 dark:text-red-200'
  return 'bg-muted text-muted-foreground'
}
function openDetail(siteId: string) {
  router.push({ path: '/sites/' + siteId })
}
function openViewer(site: ManagedProjectSite) {
  const project = encodeURIComponent(site.associated_project || site.project_name)
  window.open(`http://localhost:3101/?backendPort=${site.web_port}&output_project=${project}`, '_blank')
}
</script>

<template>
  <div class="rounded-lg border border-border">
    <div v-if="loading && props.sites.length === 0" class="p-8 text-center text-muted-foreground">
      加载中...
    </div>
    <div v-else-if="props.sites.length === 0" class="p-8 text-center text-muted-foreground">
      暂无站点，点击右上角创建
    </div>
    <table v-else class="w-full text-sm">
      <thead>
        <tr class="border-b border-border bg-muted/50">
          <th class="px-4 py-3 text-left font-medium text-muted-foreground">项目名称</th>
          <th class="px-4 py-3 text-left font-medium text-muted-foreground">状态</th>
          <th class="px-4 py-3 text-left font-medium text-muted-foreground">解析</th>
          <th class="px-4 py-3 text-left font-medium text-muted-foreground">风险</th>
          <th class="px-4 py-3 text-left font-medium text-muted-foreground">DB 端口</th>
          <th class="px-4 py-3 text-left font-medium text-muted-foreground">Web 端口</th>
          <th class="px-4 py-3 text-left font-medium text-muted-foreground">项目代码</th>
          <th class="px-4 py-3 text-right font-medium text-muted-foreground">操作</th>
        </tr>
      </thead>
      <tbody>
        <tr
          v-for="site in props.sites"
          :key="site.site_id"
          class="border-b border-border last:border-0 hover:bg-muted/30 transition-colors cursor-pointer"
          @click="openDetail(site.site_id)"
        >
          <td class="px-4 py-3 align-top">
            <div class="font-medium">{{ site.project_name }}</div>
            <div class="text-xs text-muted-foreground">{{ site.site_id }}</div>
          </td>
          <td class="px-4 py-3 align-top">
            <span class="inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium" :class="statusConfig[site.status]?.class">
              {{ statusConfig[site.status]?.label ?? site.status }}
            </span>
          </td>
          <td class="px-4 py-3 align-top">
            <span class="inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium" :class="parseStatusClass(site)">
              {{ site.parse_status }}
            </span>
          </td>
          <td class="px-4 py-3 align-top">
            <span
              class="inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium"
              :class="riskConfig[site.risk_level]?.class"
              :title="site.risk_reasons.join('；') || '当前无明显风险'"
            >
              {{ riskConfig[site.risk_level]?.label ?? site.risk_level }}
            </span>
            <div class="mt-1 max-w-[240px] truncate text-xs text-muted-foreground" :title="site.risk_reasons.join('；') || '当前无明显风险'">
              {{ riskSummary(site) }}
            </div>
          </td>
          <td class="px-4 py-3 font-mono text-muted-foreground align-top">{{ site.db_port }}</td>
          <td class="px-4 py-3 font-mono text-muted-foreground align-top">{{ site.web_port }}</td>
          <td class="px-4 py-3 text-muted-foreground align-top">{{ site.project_code }}</td>
          <td class="px-4 py-3 text-right align-top" @click.stop>
            <div class="flex items-center justify-end gap-1">
              <button
                v-if="canStart(site)"
                @click="sitesStore.startSite(site.site_id)"
                class="inline-flex h-7 w-7 items-center justify-center rounded-md hover:bg-accent transition-colors"
                title="启动"
              >
                <Play class="h-3.5 w-3.5 text-green-600" />
              </button>
              <button
                v-if="canStop(site)"
                @click="sitesStore.stopSite(site.site_id)"
                class="inline-flex h-7 w-7 items-center justify-center rounded-md hover:bg-accent transition-colors"
                title="停止"
              >
                <Square class="h-3.5 w-3.5 text-amber-600" />
              </button>
              <button
                v-if="canParse(site)"
                @click="sitesStore.parseSite(site.site_id)"
                class="inline-flex h-7 w-7 items-center justify-center rounded-md hover:bg-accent transition-colors"
                title="解析"
              >
                <RefreshCw class="h-3.5 w-3.5" />
              </button>
              <button
                v-if="site.status === 'Running'"
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
                v-if="canDelete(site)"
                @click="sitesStore.deleteSite(site.site_id)"
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
  </div>
</template>
