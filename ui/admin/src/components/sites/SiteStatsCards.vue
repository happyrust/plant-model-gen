<script setup lang="ts">
import type { SiteStats } from '@/types/site'
import { Server, Play, AlertTriangle, Clock } from 'lucide-vue-next'

defineProps<{ stats: SiteStats }>()

const cards = [
  { key: 'total' as const, label: '总站点', icon: Server, color: 'text-foreground' },
  { key: 'running' as const, label: '运行中', icon: Play, color: 'text-green-600' },
  { key: 'error' as const, label: '异常', icon: AlertTriangle, color: 'text-destructive' },
  { key: 'pending_parse' as const, label: '待解析', icon: Clock, color: 'text-amber-600' },
]
</script>

<template>
  <div class="grid gap-4 md:grid-cols-2 lg:grid-cols-4">
    <div v-for="card in cards" :key="card.key"
      class="rounded-lg border border-border bg-card p-4 transition-colors hover:bg-accent/30">
      <div class="flex items-center justify-between">
        <span class="text-sm text-muted-foreground">{{ card.label }}</span>
        <component :is="card.icon" class="h-4 w-4 text-muted-foreground" />
      </div>
      <div class="mt-2 text-2xl font-bold" :class="card.color">{{ stats[card.key] }}</div>
    </div>
  </div>
</template>
