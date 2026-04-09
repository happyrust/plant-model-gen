<script setup lang="ts">
import { useRouter } from 'vue-router'
import { useSitesStore } from '@/stores/sites'
import type { ManagedProjectSite, ManagedSiteStatus } from '@/types/site'
import { Play, Square, RefreshCw, Trash2, ExternalLink } from 'lucide-vue-next'

defineProps<{ sites: ManagedProjectSite[]; loading: boolean }>()

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

function canStart(site: ManagedProjectSite) {
  return ['Stopped', 'Parsed', 'Failed', 'Draft'].includes(site.status)
}
function canStop(site: ManagedProjectSite) {
  return site.status === 'Running'
}
function canParse(site: ManagedProjectSite) {
  return !['Starting', 'Stopping'].includes(site.status) && site.parse_status !== 'Running'
}
function canDelete(site: ManagedProjectSite) {
  return !['Running', 'Starting', 'Stopping'].includes(site.status)
}
</script>

<template>
  <div class="rounded-lg border border-border">
    <div v-if="loading && !sites.length" class="p-8 text-center text-muted-foreground">
      加载中...
    </div>
    <div v-else-if="!sites.length" class="p-8 text-center text-muted-foreground">
      暂无站点，点击右上角创建
    </div>
    <table v-else class="w-full text-sm">
      <thead>
        <tr class="border-b border-border bg-muted/50">
          <th class="px-4 py-3 text-left font-medium text-muted-foreground">项目名称</th>
          <th class="px-4 py-3 text-left font-medium text-muted-foreground">状态</th>
          <th class="px-4 py-3 text-left font-medium text-muted-foreground">解析</th>
          <th class="px-4 py-3 text-left font-medium text-muted-foreground">DB 端口</th>
          <th class="px-4 py-3 text-left font-medium text-muted-foreground">Web 端口</th>
          <th class="px-4 py-3 text-left font-medium text-muted-foreground">项目代码</th>
          <th class="px-4 py-3 text-right font-medium text-muted-foreground">操作</th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="site in sites" :key="site.site_id"
          class="border-b border-border last:border-0 hover:bg-muted/30 transition-colors cursor-pointer"
          @click="router.push(`/sites/${site.site_id}`)">
          <td class="px-4 py-3">
            <div class="font-medium">{{ site.project_name }}</div>
            <div class="text-xs text-muted-foreground">{{ site.site_id }}</div>
          </td>
          <td class="px-4 py-3">
            <span class="inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium"
              :class="statusConfig[site.status]?.class">
              {{ statusConfig[site.status]?.label ?? site.status }}
            </span>
          </td>
          <td class="px-4 py-3">
            <span class="inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium"
              :class="site.parse_status === 'Parsed' ? 'bg-green-100 text-green-800' :
                       site.parse_status === 'Running' ? 'bg-blue-100 text-blue-800' :
                       site.parse_status === 'Failed' ? 'bg-red-100 text-red-800' :
                       'bg-muted text-muted-foreground'">
              {{ site.parse_status }}
            </span>
          </td>
          <td class="px-4 py-3 font-mono text-muted-foreground">{{ site.db_port }}</td>
          <td class="px-4 py-3 font-mono text-muted-foreground">{{ site.web_port }}</td>
          <td class="px-4 py-3 text-muted-foreground">{{ site.project_code }}</td>
          <td class="px-4 py-3 text-right" @click.stop>
            <div class="flex items-center justify-end gap-1">
              <button v-if="canStart(site)"
                @click="sitesStore.startSite(site.site_id)"
                class="inline-flex h-7 w-7 items-center justify-center rounded-md hover:bg-accent transition-colors"
                title="启动">
                <Play class="h-3.5 w-3.5 text-green-600" />
              </button>
              <button v-if="canStop(site)"
                @click="sitesStore.stopSite(site.site_id)"
                class="inline-flex h-7 w-7 items-center justify-center rounded-md hover:bg-accent transition-colors"
                title="停止">
                <Square class="h-3.5 w-3.5 text-amber-600" />
              </button>
              <button v-if="canParse(site)"
                @click="sitesStore.parseSite(site.site_id)"
                class="inline-flex h-7 w-7 items-center justify-center rounded-md hover:bg-accent transition-colors"
                title="解析">
                <RefreshCw class="h-3.5 w-3.5" />
              </button>
              <button @click="router.push(`/sites/${site.site_id}`)"
                class="inline-flex h-7 w-7 items-center justify-center rounded-md hover:bg-accent transition-colors"
                title="详情">
                <ExternalLink class="h-3.5 w-3.5" />
              </button>
              <button v-if="canDelete(site)"
                @click="sitesStore.deleteSite(site.site_id)"
                class="inline-flex h-7 w-7 items-center justify-center rounded-md hover:bg-accent transition-colors"
                title="删除">
                <Trash2 class="h-3.5 w-3.5 text-destructive" />
              </button>
            </div>
          </td>
        </tr>
      </tbody>
    </table>
  </div>
</template>
