<script setup lang="ts">
import type { CollaborationOverviewMetric, CollaborationTone } from '@/types/collaboration'

defineProps<{
  metrics: CollaborationOverviewMetric[]
  loading: boolean
}>()

function toneClass(tone: CollaborationTone) {
  switch (tone) {
    case 'success':
      return 'text-emerald-600'
    case 'warning':
      return 'text-amber-600'
    case 'danger':
      return 'text-rose-600'
    default:
      return 'text-foreground'
  }
}
</script>

<template>
  <section class="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
    <div
      v-for="metric in metrics"
      :key="metric.id"
      class="rounded-xl border border-border bg-card px-5 py-4"
    >
      <div v-if="loading" class="space-y-2">
        <div class="h-3 w-16 animate-pulse rounded bg-muted" />
        <div class="h-7 w-24 animate-pulse rounded bg-muted" />
        <div class="h-3 w-32 animate-pulse rounded bg-muted" />
      </div>
      <template v-else>
        <div class="text-sm text-muted-foreground">{{ metric.label }}</div>
        <div class="mt-2 text-2xl font-semibold tracking-tight" :class="toneClass(metric.tone)">
          {{ metric.value }}
        </div>
        <div class="mt-2 text-xs text-muted-foreground">{{ metric.detail }}</div>
      </template>
    </div>
  </section>
</template>
