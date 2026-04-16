<script setup lang="ts">
import { Clock, Activity } from 'lucide-vue-next'
import type { ManagedSiteRuntimeStatus } from '@/types/site'

defineProps<{
  runtime: ManagedSiteRuntimeStatus | null
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
  <div v-if="runtime?.recent_activity || runtime?.last_key_log" class="rounded-lg border border-border bg-card p-5">
    <div class="mb-4 flex items-center gap-2">
      <Activity class="h-4 w-4 text-muted-foreground" />
      <h3 class="text-base font-medium">最近活动</h3>
    </div>
    <div class="space-y-3">
      <div v-if="runtime.recent_activity" class="rounded-lg border border-border/60 bg-background p-4">
        <div class="flex items-center justify-between">
          <span class="text-sm font-medium">{{ runtime.recent_activity.label }}</span>
          <span class="flex items-center gap-1 text-xs text-muted-foreground">
            <Clock class="h-3 w-3" />
            {{ relativeTime(runtime.recent_activity.updated_at) }}
          </span>
        </div>
        <div v-if="runtime.recent_activity.summary" class="mt-2 text-sm text-muted-foreground">
          {{ runtime.recent_activity.summary }}
        </div>
      </div>
      <div v-if="runtime.last_key_log" class="rounded-lg border border-border/60 bg-background p-4">
        <div class="flex items-center justify-between">
          <span class="text-xs text-muted-foreground">关键日志</span>
          <span class="flex items-center gap-1 text-xs text-muted-foreground">
            <Clock class="h-3 w-3" />
            {{ relativeTime(runtime.recent_log_at) }}
          </span>
        </div>
        <div class="mt-1 text-sm font-mono text-muted-foreground truncate" :title="runtime.last_key_log">
          {{ runtime.last_key_log }}
        </div>
        <div v-if="runtime.last_key_log_source" class="mt-1 text-xs text-muted-foreground">
          来源: {{ runtime.last_key_log_source }}
        </div>
      </div>
    </div>
  </div>
</template>
