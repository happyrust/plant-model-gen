<script setup lang="ts">
import { FileText, Clock } from 'lucide-vue-next'
import type { ManagedSiteLogStreamSummary } from '@/types/site'

defineProps<{
  streams: ManagedSiteLogStreamSummary[]
}>()

function relativeTime(iso?: string | null) {
  if (!iso) return '-'
  const diff = Date.now() - new Date(iso).getTime()
  if (Number.isNaN(diff)) return '-'
  if (diff < 60_000) return '刚刚'
  if (diff < 3_600_000) return Math.floor(diff / 60_000) + ' 分钟前'
  if (diff < 86_400_000) return Math.floor(diff / 3_600_000) + ' 小时前'
  return Math.floor(diff / 86_400_000) + ' 天前'
}
</script>

<template>
  <div v-if="streams.length" class="rounded-lg border border-border bg-card p-5">
    <div class="mb-4 flex items-center gap-2">
      <FileText class="h-4 w-4 text-muted-foreground" />
      <h3 class="text-base font-medium">日志摘要</h3>
    </div>
    <div class="grid gap-3 md:grid-cols-3">
      <div
        v-for="stream in streams"
        :key="stream.key"
        class="rounded-lg border border-border/60 bg-background p-4"
      >
        <div class="flex items-center justify-between">
          <span class="text-sm font-medium">{{ stream.label }}</span>
          <span
            class="inline-flex items-center rounded-full px-1.5 py-0.5 text-[10px] font-medium"
            :class="stream.has_content ? 'bg-blue-100 text-blue-700 dark:bg-blue-900 dark:text-blue-300' : 'bg-muted text-muted-foreground'"
          >
            {{ stream.line_count }} 行
          </span>
        </div>
        <div class="mt-2 flex items-center gap-1 text-xs text-muted-foreground">
          <Clock class="h-3 w-3" />
          <span>{{ relativeTime(stream.updated_at) }}</span>
        </div>
        <div
          v-if="stream.last_key_log"
          class="mt-2 truncate text-xs text-muted-foreground"
          :title="stream.last_key_log"
        >
          {{ stream.last_key_log }}
        </div>
        <div v-else class="mt-2 text-xs text-muted-foreground italic">暂无关键日志</div>
      </div>
    </div>
  </div>
</template>
