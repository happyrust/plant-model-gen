<script setup lang="ts">
import { computed } from 'vue'
import { formatBytes, formatDateTime } from '@/lib/collaboration'
import type { CollaborationFlowStat, CollaborationInsightsSummary } from '@/types/collaboration'

const props = defineProps<{
  summary: CollaborationInsightsSummary
  loading: boolean
  error: string
}>()

const trendItems = computed(() => {
  const maxTotal = Math.max(...props.summary.trend14d.map((item) => item.total), 0)
  return props.summary.trend14d.map((item) => ({
    ...item,
    height: maxTotal > 0 ? Math.max(10, Math.round((item.total / maxTotal) * 100)) : 10,
    label: item.day.slice(5),
  }))
})

function describeFlow(flow: CollaborationFlowStat | null) {
  if (!flow) return '暂无流向数据'
  const targetSite = flow.target_site || '未命名站点'
  const direction = flow.direction || '未知方向'
  return `${targetSite} · ${direction}`
}
</script>

<template>
  <section class="rounded-xl border border-border bg-card">
    <div class="border-b border-border px-5 py-4">
      <p class="text-xs font-medium uppercase tracking-[0.18em] text-muted-foreground">同步洞察卡片</p>
      <h2 class="mt-1 text-lg font-semibold">趋势、异常与失败流向</h2>
      <p class="mt-1 text-sm text-muted-foreground">基于最近 14 天统计与当前日志窗口，帮助快速判断异常集中区。</p>
    </div>

    <div class="p-5">
      <div
        v-if="error"
        class="mb-4 rounded-lg border border-amber-500/20 bg-amber-500/5 px-4 py-3 text-sm text-amber-700"
      >
        {{ error }}
      </div>

      <div v-if="loading" class="space-y-4">
        <div class="grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
          <div v-for="index in 4" :key="index" class="h-24 animate-pulse rounded-xl border border-border bg-muted/50" />
        </div>
        <div class="h-52 animate-pulse rounded-xl border border-border bg-muted/50" />
        <div class="grid gap-4 xl:grid-cols-2">
          <div class="h-56 animate-pulse rounded-xl border border-border bg-muted/50" />
          <div class="h-56 animate-pulse rounded-xl border border-border bg-muted/50" />
        </div>
      </div>

      <template v-else>
        <div class="grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
          <div class="rounded-xl border border-border bg-background p-4">
            <div class="text-sm text-muted-foreground">近 7 天同步量</div>
            <div class="mt-2 text-2xl font-semibold">{{ summary.total7d }}</div>
            <div class="mt-1 text-xs text-muted-foreground">近 14 天累计 {{ summary.total14d }}</div>
          </div>
          <div class="rounded-xl border border-border bg-background p-4">
            <div class="text-sm text-muted-foreground">成功率</div>
            <div
              class="mt-2 text-2xl font-semibold"
              :class="summary.successRate >= 0.9 ? 'text-emerald-600' : summary.successRate >= 0.7 ? 'text-amber-600' : 'text-rose-600'"
            >
              {{ Math.round(summary.successRate * 100) }}%
            </div>
            <div class="mt-1 text-xs text-muted-foreground">成功 {{ summary.completed }} · 失败 {{ summary.failed }}</div>
          </div>
          <div class="rounded-xl border border-border bg-background p-4">
            <div class="text-sm text-muted-foreground">总记录 / 总流量</div>
            <div class="mt-2 text-2xl font-semibold">{{ summary.totalRecords }}</div>
            <div class="mt-1 text-xs text-muted-foreground">{{ formatBytes(summary.totalBytes) }}</div>
          </div>
          <div class="rounded-xl border border-border bg-background p-4">
            <div class="text-sm text-muted-foreground">最近同步活跃度</div>
            <div class="mt-2 text-2xl font-semibold" :class="summary.lastLogAt ? 'text-foreground' : 'text-muted-foreground'">
              {{ summary.lastLogAt ? '有活动' : '暂无' }}
            </div>
            <div class="mt-1 text-xs text-muted-foreground">{{ formatDateTime(summary.lastLogAt) }}</div>
          </div>
        </div>

        <div class="mt-4 rounded-xl border border-border bg-background p-4">
          <div class="flex items-center justify-between gap-3">
            <div>
              <div class="text-sm font-medium text-foreground">近 14 天趋势条</div>
              <div class="mt-1 text-xs text-muted-foreground">按每日同步总量显示。</div>
            </div>
            <div class="text-xs text-muted-foreground">共 {{ summary.trend14d.length }} 天</div>
          </div>

          <div
            v-if="!trendItems.length"
            class="mt-4 rounded-lg border border-dashed border-border bg-muted/20 px-4 py-10 text-center text-sm text-muted-foreground"
          >
            近 14 天没有统计数据。
          </div>

          <div v-else class="mt-4 flex h-44 items-end gap-2 overflow-hidden">
            <div
              v-for="item in trendItems"
              :key="item.day"
              class="flex min-w-0 flex-1 flex-col items-center gap-2"
            >
              <div class="w-full rounded-t-md bg-primary/80" :style="{ height: `${item.height}%` }" />
              <div class="text-[11px] text-muted-foreground">{{ item.label }}</div>
            </div>
          </div>
        </div>

        <div class="mt-4 grid gap-4 xl:grid-cols-2">
          <div class="rounded-xl border border-border bg-background p-4">
            <div class="flex items-center justify-between gap-3">
              <div>
                <div class="text-sm font-medium text-foreground">失败流向前 5</div>
                <div class="mt-1 text-xs text-muted-foreground">按失败次数排序。</div>
              </div>
              <div class="text-xs text-muted-foreground">告警 {{ summary.alertCount }}</div>
            </div>

            <div
              v-if="!summary.topFailedFlows.length"
              class="mt-4 rounded-lg border border-dashed border-border bg-muted/20 px-4 py-8 text-center text-sm text-muted-foreground"
            >
              当前没有失败流向。
            </div>

            <div v-else class="mt-4 space-y-3">
              <div
                v-for="flow in summary.topFailedFlows"
                :key="`${flow.target_site}-${flow.direction}`"
                class="rounded-lg border border-border px-3 py-3"
              >
                <div class="flex items-start justify-between gap-3">
                  <div>
                    <div class="text-sm font-medium text-foreground">{{ describeFlow(flow) }}</div>
                    <div class="mt-1 text-xs text-muted-foreground">
                      总量 {{ flow.total }} · 记录 {{ flow.record_count }} · {{ formatBytes(flow.total_bytes) }}
                    </div>
                  </div>
                  <div class="rounded-full border border-rose-500/20 bg-rose-500/5 px-2 py-0.5 text-xs font-medium text-rose-600">
                    失败 {{ flow.failed }}
                  </div>
                </div>
              </div>
            </div>
          </div>

          <div class="rounded-xl border border-border bg-background p-4">
            <div class="flex items-center justify-between gap-3">
              <div>
                <div class="text-sm font-medium text-foreground">最近异常摘要</div>
                <div class="mt-1 text-xs text-muted-foreground">取当前日志窗口中的最近失败记录。</div>
              </div>
              <div class="text-xs text-muted-foreground">最近 {{ summary.recentFailures.length }} 条</div>
            </div>

            <div
              v-if="!summary.recentFailures.length"
              class="mt-4 rounded-lg border border-dashed border-border bg-muted/20 px-4 py-8 text-center text-sm text-muted-foreground"
            >
              当前日志窗口中没有失败记录。
            </div>

            <div v-else class="mt-4 space-y-3">
              <div
                v-for="log in summary.recentFailures"
                :key="log.id"
                class="rounded-lg border border-border px-3 py-3"
              >
                <div class="flex items-start justify-between gap-3">
                  <div class="min-w-0">
                    <div class="text-sm font-medium text-foreground">
                      {{ log.target_site || '未命名站点' }} · {{ log.direction || '未知方向' }}
                    </div>
                    <div class="mt-1 truncate text-xs text-muted-foreground">
                      {{ log.error_message || log.notes || log.file_path || '无异常详情' }}
                    </div>
                  </div>
                  <div class="shrink-0 text-xs text-muted-foreground">{{ formatDateTime(log.created_at) }}</div>
                </div>
              </div>
            </div>
          </div>
        </div>
      </template>
    </div>
  </section>
</template>
