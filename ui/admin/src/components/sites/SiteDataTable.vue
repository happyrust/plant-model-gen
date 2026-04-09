<script setup lang="ts">
import { useRouter } from 'vue-router'
import { useSitesStore } from '@/stores/sites'
import type { Site } from '@/types/site'
import { Play, Square, RefreshCw, Trash2, ExternalLink } from 'lucide-vue-next'

defineProps<{ sites: Site[]; loading: boolean }>()

const router = useRouter()
const sitesStore = useSitesStore()

const statusColors: Record<string, string> = {
  running: 'bg-green-100 text-green-800 dark:bg-green-900 dark:text-green-200',
  stopped: 'bg-muted text-muted-foreground',
  error: 'bg-red-100 text-red-800 dark:bg-red-900 dark:text-red-200',
  parsing: 'bg-blue-100 text-blue-800 dark:bg-blue-900 dark:text-blue-200',
  pending: 'bg-amber-100 text-amber-800 dark:bg-amber-900 dark:text-amber-200',
}

const envLabels: Record<string, string> = {
  dev: '开发',
  staging: '预发布',
  prod: '生产',
}
</script>

<template>
  <div class="rounded-lg border border-border">
    <div v-if="loading && !sites.length" class="p-8 text-center text-muted-foreground">
      加载中...
    </div>
    <div v-else-if="!sites.length" class="p-8 text-center text-muted-foreground">
      暂无站点
    </div>
    <table v-else class="w-full text-sm">
      <thead>
        <tr class="border-b border-border bg-muted/50">
          <th class="px-4 py-3 text-left font-medium text-muted-foreground">名称</th>
          <th class="px-4 py-3 text-left font-medium text-muted-foreground">环境</th>
          <th class="px-4 py-3 text-left font-medium text-muted-foreground">状态</th>
          <th class="px-4 py-3 text-left font-medium text-muted-foreground">端口</th>
          <th class="px-4 py-3 text-right font-medium text-muted-foreground">操作</th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="site in sites" :key="site.id"
          class="border-b border-border last:border-0 hover:bg-muted/30 transition-colors cursor-pointer"
          @click="router.push(`/sites/${site.id}`)">
          <td class="px-4 py-3 font-medium">{{ site.name }}</td>
          <td class="px-4 py-3">
            <span class="inline-flex items-center rounded-md border px-2 py-0.5 text-xs font-medium">
              {{ envLabels[site.environment] ?? site.environment }}
            </span>
          </td>
          <td class="px-4 py-3">
            <span class="inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium"
              :class="statusColors[site.status]">{{ site.status }}</span>
          </td>
          <td class="px-4 py-3 text-muted-foreground">{{ site.port }}</td>
          <td class="px-4 py-3 text-right" @click.stop>
            <div class="flex items-center justify-end gap-1">
              <button v-if="site.status === 'stopped' || site.status === 'error'"
                @click="sitesStore.startSite(site.id)"
                class="inline-flex h-7 w-7 items-center justify-center rounded-md hover:bg-accent transition-colors"
                title="启动">
                <Play class="h-3.5 w-3.5 text-green-600" />
              </button>
              <button v-if="site.status === 'running'"
                @click="sitesStore.stopSite(site.id)"
                class="inline-flex h-7 w-7 items-center justify-center rounded-md hover:bg-accent transition-colors"
                title="停止">
                <Square class="h-3.5 w-3.5 text-amber-600" />
              </button>
              <button @click="sitesStore.parseSite(site.id)"
                class="inline-flex h-7 w-7 items-center justify-center rounded-md hover:bg-accent transition-colors"
                title="解析">
                <RefreshCw class="h-3.5 w-3.5" />
              </button>
              <button @click="router.push(`/sites/${site.id}`)"
                class="inline-flex h-7 w-7 items-center justify-center rounded-md hover:bg-accent transition-colors"
                title="详情">
                <ExternalLink class="h-3.5 w-3.5" />
              </button>
              <button @click="sitesStore.deleteSite(site.id)"
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
