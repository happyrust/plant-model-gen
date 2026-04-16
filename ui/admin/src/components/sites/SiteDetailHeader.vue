<script setup lang="ts">
import { ArrowLeft, ExternalLink, Loader2, Play, RefreshCw, Square } from 'lucide-vue-next'
import type { ManagedProjectSite } from '@/types/site'
import { statusLabelMap, statusClassMap, isSiteBusy } from './site-status'
import { useSitesStore } from '@/stores/sites'

const props = defineProps<{
  site: ManagedProjectSite | null
  viewerUrl: string | null
}>()

const sitesStore = useSitesStore()

function isPending() {
  return props.site ? sitesStore.isSiteActionPending(props.site.site_id) : false
}
function actionLabel() {
  if (!props.site) return ''
  const action = sitesStore.getSiteAction(props.site.site_id)
  if (action === 'start') return '启动中...'
  if (action === 'stop') return '停止中...'
  if (action === 'parse') return '解析中...'
  return '处理中...'
}

defineEmits<{
  back: []
  start: []
  stop: []
  parse: []
  refresh: []
  openViewer: []
}>()

function canStart() {
  const s = props.site
  return s && !isSiteBusy(s) && ['Stopped', 'Parsed', 'Failed', 'Draft'].includes(s.status)
}
function canStop() {
  return props.site?.status === 'Running'
}
function canParse() {
  return props.site && !isSiteBusy(props.site)
}
</script>

<template>
  <div class="flex items-center gap-4">
    <button
      @click="$emit('back')"
      class="inline-flex h-8 w-8 items-center justify-center rounded-md hover:bg-accent transition-colors"
    >
      <ArrowLeft class="h-4 w-4" />
    </button>
    <div v-if="site" class="flex-1 min-w-0">
      <div class="flex items-center gap-3">
        <h2 class="text-2xl font-semibold tracking-tight truncate">{{ site.project_name }}</h2>
        <span
          class="inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium shrink-0"
          :class="statusClassMap[site.status]"
        >
          {{ statusLabelMap[site.status] }}
        </span>
      </div>
      <p class="text-sm text-muted-foreground truncate">{{ site.site_id }}</p>
    </div>
    <div v-if="site" class="flex items-center gap-2 shrink-0">
      <div v-if="isPending()" class="flex items-center gap-2 text-sm text-muted-foreground">
        <Loader2 class="h-4 w-4 animate-spin" />
        <span>{{ actionLabel() }}</span>
      </div>
      <template v-else>
        <button
          @click="$emit('refresh')"
          class="inline-flex h-9 items-center gap-2 rounded-md border border-input bg-transparent px-3 text-sm font-medium hover:bg-accent transition-colors"
          title="刷新"
        >
          <RefreshCw class="h-4 w-4" />
        </button>
        <button
          v-if="canParse()"
          @click="$emit('parse')"
          class="inline-flex h-9 items-center gap-2 rounded-md border border-input bg-transparent px-3 text-sm font-medium hover:bg-accent transition-colors"
        >
          <RefreshCw class="h-4 w-4" /> 解析
        </button>
        <button
          v-if="canStart()"
          @click="$emit('start')"
          class="inline-flex h-9 items-center gap-2 rounded-md bg-green-600 px-4 text-sm font-medium text-white shadow hover:bg-green-700 transition-colors"
        >
          <Play class="h-4 w-4" /> 启动
        </button>
        <button
          v-if="canStop()"
          @click="$emit('stop')"
          class="inline-flex h-9 items-center gap-2 rounded-md bg-amber-600 px-4 text-sm font-medium text-white shadow hover:bg-amber-700 transition-colors"
        >
          <Square class="h-4 w-4" /> 停止
        </button>
        <button
          v-if="site.status === 'Running' && viewerUrl"
          @click="$emit('openViewer')"
          class="inline-flex h-9 items-center gap-2 rounded-md bg-primary px-4 text-sm font-medium text-primary-foreground shadow hover:bg-primary/90 transition-colors"
        >
          <ExternalLink class="h-4 w-4" /> Viewer
        </button>
      </template>
    </div>
  </div>
</template>
