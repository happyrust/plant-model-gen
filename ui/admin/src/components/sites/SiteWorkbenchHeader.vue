<script setup lang="ts">
import { computed } from 'vue'
import { RadioTower, RefreshCw } from 'lucide-vue-next'

const props = defineProps<{
  total: number
  filtered: number
  lastRefresh: string | null
  refreshing: boolean
  realtimeConnected: boolean
  reconnectAttempt: number
}>()

const emit = defineEmits<{
  refresh: []
}>()

const refreshLabel = computed(() => {
  if (!props.lastRefresh) return ''
  const d = new Date(props.lastRefresh)
  if (Number.isNaN(d.getTime())) return ''
  return d.toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit', second: '2-digit' })
})

const realtimeTitle = computed(() => {
  if (props.realtimeConnected) return '已订阅 /api/sync/events，状态变更将实时推送'
  if (props.reconnectAttempt > 0) return `SSE 断流，正在指数退避重连（第 ${props.reconnectAttempt} 次）`
  return 'SSE 未连接，列表依赖 60s 兜底刷新'
})
</script>

<template>
  <div class="flex flex-wrap items-start justify-between gap-4">
    <div>
      <h2 class="text-2xl font-semibold tracking-tight">站点管理</h2>
      <p class="text-sm text-muted-foreground">本机多站点编排工作台</p>
    </div>
    <div class="flex flex-wrap items-center justify-end gap-2">
      <div
        class="inline-flex h-7 items-center gap-1.5 rounded-full px-3 text-xs font-medium"
        :class="realtimeConnected
          ? 'bg-emerald-500/10 text-emerald-700 dark:text-emerald-400'
          : reconnectAttempt > 0
            ? 'bg-amber-500/10 text-amber-700 dark:text-amber-400'
            : 'bg-muted text-muted-foreground'"
        :title="realtimeTitle"
      >
        <RadioTower class="h-3.5 w-3.5" />
        <span v-if="realtimeConnected">实时已连接</span>
        <span v-else-if="reconnectAttempt > 0">重连中 #{{ reconnectAttempt }}</span>
        <span v-else>实时未连接</span>
      </div>
      <div v-if="refreshLabel" class="text-xs text-muted-foreground">
        最近刷新 {{ refreshLabel }}
      </div>
      <div class="text-xs text-muted-foreground">
        {{ filtered === total ? `共 ${total} 个站点` : `${filtered} / ${total} 个站点` }}
      </div>
      <button
        @click="emit('refresh')"
        :disabled="refreshing"
        class="inline-flex h-9 items-center gap-2 rounded-md border border-input px-4 text-sm font-medium hover:bg-accent transition-colors disabled:opacity-50"
      >
        <RefreshCw class="h-4 w-4" :class="{ 'animate-spin': refreshing }" />
        刷新
      </button>
    </div>
  </div>
</template>
