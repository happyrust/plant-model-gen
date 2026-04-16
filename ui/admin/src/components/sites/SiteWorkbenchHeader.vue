<script setup lang="ts">
import { computed } from 'vue'
import { RefreshCw } from 'lucide-vue-next'

const props = defineProps<{
  total: number
  filtered: number
  lastRefresh: string | null
  refreshing: boolean
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
</script>

<template>
  <div class="flex flex-wrap items-start justify-between gap-4">
    <div>
      <h2 class="text-2xl font-semibold tracking-tight">站点管理</h2>
      <p class="text-sm text-muted-foreground">本机多站点编排工作台</p>
    </div>
    <div class="flex items-center gap-3">
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
