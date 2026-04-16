<script setup lang="ts">
import { computed, ref } from 'vue'
import { formatBytes, formatDateTime } from '@/lib/collaboration'
import type {
  CollaborationLogFilters,
  CollaborationLogRecord,
  CollaborationOption,
} from '@/types/collaboration'

const props = defineProps<{
  logs: CollaborationLogRecord[]
  total: number
  loading: boolean
  error: string
  filters: CollaborationLogFilters
  targetSiteOptions: CollaborationOption[]
  directionOptions: CollaborationOption[]
  statusOptions: CollaborationOption[]
}>()

const emit = defineEmits<{
  updateFilters: [filters: Partial<CollaborationLogFilters>]
}>()

const expandedErrorIds = ref<string[]>([])

const mergedStatusOptions = computed(() =>
  props.statusOptions.some((item) => item.value === props.filters.status) || props.filters.status === ''
    ? props.statusOptions
    : [...props.statusOptions, { value: props.filters.status, label: props.filters.status }],
)

const mergedDirectionOptions = computed(() =>
  props.directionOptions.some((item) => item.value === props.filters.direction) || props.filters.direction === ''
    ? props.directionOptions
    : [...props.directionOptions, { value: props.filters.direction, label: props.filters.direction }],
)

const mergedTargetSiteOptions = computed(() =>
  props.targetSiteOptions.some((item) => item.value === props.filters.target_site) || props.filters.target_site === ''
    ? props.targetSiteOptions
    : [...props.targetSiteOptions, { value: props.filters.target_site, label: props.filters.target_site }],
)

const filteredLogs = computed(() => {
  const keyword = props.filters.keyword.trim().toLowerCase()
  if (!keyword) return props.logs

  return props.logs.filter((log) => {
    const haystacks = [
      log.task_id,
      log.target_site,
      log.file_path,
      log.error_message,
      log.notes,
    ]

    return haystacks.some((value) => value?.toLowerCase().includes(keyword))
  })
})

const summary = computed(() => {
  const failureCount = filteredLogs.value.filter((log) => log.status === 'failed').length
  const successCount = filteredLogs.value.filter((log) => log.status === 'completed').length
  return {
    currentCount: filteredLogs.value.length,
    failureCount,
    successCount,
    latestAt: filteredLogs.value[0]?.created_at ?? null,
  }
})

function statusClass(status: string) {
  if (status === 'completed') return 'bg-emerald-500/10 text-emerald-600 border-emerald-500/20'
  if (status === 'failed') return 'bg-rose-500/10 text-rose-600 border-rose-500/20'
  return 'bg-amber-500/10 text-amber-600 border-amber-500/20'
}

function recordSummary(log: CollaborationLogRecord) {
  const parts = []
  if (log.record_count !== null) parts.push(`记录 ${log.record_count}`)
  if (log.file_size !== null) parts.push(formatBytes(log.file_size))
  return parts.join(' · ') || '无记录摘要'
}

function isExpanded(id: string) {
  return expandedErrorIds.value.includes(id)
}

function isLongError(message: string) {
  return message.length > 160 || message.includes('\n')
}

function toggleError(id: string) {
  if (isExpanded(id)) {
    expandedErrorIds.value = expandedErrorIds.value.filter((item) => item !== id)
    return
  }
  expandedErrorIds.value = [...expandedErrorIds.value, id]
}

function displayError(log: CollaborationLogRecord) {
  const message = log.error_message || ''
  if (!isLongError(message) || isExpanded(log.id)) return message
  return message.slice(0, 160) + '...'
}
</script>

<template>
  <section class="rounded-xl border border-border bg-card">
    <div class="border-b border-border px-5 py-4">
      <div class="flex flex-col gap-4 lg:flex-row lg:items-end lg:justify-between">
        <div>
          <p class="text-xs font-medium uppercase tracking-[0.18em] text-muted-foreground">实时同步日志卡片</p>
          <h2 class="mt-1 text-lg font-semibold">最近关键日志</h2>
          <p class="mt-1 text-sm text-muted-foreground">
            总计 {{ total }} 条，当前窗口 {{ logs.length }} 条，关键词后 {{ filteredLogs.length }} 条。
          </p>
        </div>

        <div class="grid gap-2 sm:grid-cols-2 xl:grid-cols-4">
          <select
            class="h-9 rounded-md border border-input bg-background px-3 text-sm outline-none transition-colors focus:border-primary"
            :value="filters.status"
            @change="emit('updateFilters', { status: ($event.target as HTMLSelectElement).value })"
          >
            <option v-for="item in mergedStatusOptions" :key="item.value" :value="item.value">{{ item.label }}</option>
          </select>
          <select
            class="h-9 rounded-md border border-input bg-background px-3 text-sm outline-none transition-colors focus:border-primary"
            :value="filters.direction"
            @change="emit('updateFilters', { direction: ($event.target as HTMLSelectElement).value })"
          >
            <option v-for="item in mergedDirectionOptions" :key="item.value" :value="item.value">{{ item.label }}</option>
          </select>
          <select
            class="h-9 rounded-md border border-input bg-background px-3 text-sm outline-none transition-colors focus:border-primary"
            :value="filters.target_site"
            @change="emit('updateFilters', { target_site: ($event.target as HTMLSelectElement).value })"
          >
            <option v-for="item in mergedTargetSiteOptions" :key="item.value" :value="item.value">{{ item.label }}</option>
          </select>
          <input
            class="h-9 rounded-md border border-input bg-background px-3 text-sm outline-none transition-colors focus:border-primary"
            type="text"
            placeholder="关键词搜索"
            :value="filters.keyword"
            @input="emit('updateFilters', { keyword: ($event.target as HTMLInputElement).value })"
          />
        </div>
      </div>
    </div>

    <div class="p-5">
      <div
        v-if="error"
        class="mb-4 rounded-lg border border-amber-500/20 bg-amber-500/5 px-4 py-3 text-sm text-amber-700"
      >
        {{ error }}
      </div>

      <div class="mb-4 grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
        <div class="rounded-xl border border-border bg-background p-4">
          <div class="text-sm text-muted-foreground">当前结果数</div>
          <div class="mt-2 text-2xl font-semibold">{{ summary.currentCount }}</div>
        </div>
        <div class="rounded-xl border border-border bg-background p-4">
          <div class="text-sm text-muted-foreground">失败数</div>
          <div class="mt-2 text-2xl font-semibold text-rose-600">{{ summary.failureCount }}</div>
        </div>
        <div class="rounded-xl border border-border bg-background p-4">
          <div class="text-sm text-muted-foreground">成功数</div>
          <div class="mt-2 text-2xl font-semibold text-emerald-600">{{ summary.successCount }}</div>
        </div>
        <div class="rounded-xl border border-border bg-background p-4">
          <div class="text-sm text-muted-foreground">最近一条日志时间</div>
          <div class="mt-2 text-sm font-semibold text-foreground">{{ formatDateTime(summary.latestAt) }}</div>
        </div>
      </div>

      <div v-if="loading" class="space-y-3">
        <div
          v-for="index in 4"
          :key="index"
          class="h-28 animate-pulse rounded-xl border border-border bg-muted/50"
        />
      </div>

      <div
        v-else-if="!filteredLogs.length"
        class="rounded-xl border border-dashed border-border bg-muted/20 px-4 py-10 text-center text-sm text-muted-foreground"
      >
        当前筛选条件下没有同步日志。
      </div>

      <div v-else class="space-y-3">
        <article
          v-for="log in filteredLogs"
          :key="log.id"
          class="rounded-xl border border-border bg-background p-4"
        >
          <div class="flex flex-col gap-3 lg:flex-row lg:items-start lg:justify-between">
            <div class="min-w-0 flex-1">
              <div class="flex flex-wrap items-center gap-2">
                <span
                  class="inline-flex items-center rounded-full border px-2 py-0.5 text-xs font-medium"
                  :class="statusClass(log.status)"
                >
                  {{ log.status }}
                </span>
                <span class="text-sm font-medium text-foreground">
                  {{ log.target_site || '未命名站点' }}
                </span>
                <span class="text-xs text-muted-foreground">{{ log.direction || '未知方向' }}</span>
                <span v-if="log.task_id" class="text-xs text-muted-foreground">任务 {{ log.task_id }}</span>
              </div>
              <div class="mt-2 break-all font-mono text-xs text-muted-foreground">
                {{ log.file_path || log.notes || '无文件路径' }}
              </div>
              <div class="mt-3 text-xs text-muted-foreground">
                {{ recordSummary(log) }}
              </div>
            </div>
            <div class="shrink-0 text-xs text-muted-foreground">
              {{ formatDateTime(log.created_at) }}
            </div>
          </div>

          <div
            v-if="log.error_message"
            class="mt-4 rounded-lg border border-rose-500/20 bg-rose-500/5 px-3 py-2 text-xs text-rose-600"
          >
            <div class="whitespace-pre-wrap break-words">{{ displayError(log) }}</div>
            <button
              v-if="isLongError(log.error_message)"
              class="mt-2 text-xs font-medium text-rose-700 underline underline-offset-2"
              @click="toggleError(log.id)"
            >
              {{ isExpanded(log.id) ? '收起' : '展开' }}
            </button>
          </div>
        </article>
      </div>
    </div>
  </section>
</template>
