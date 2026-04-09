<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { useRoute } from 'vue-router'
import { sitesApi } from '@/api/sites'
import type { Site, SiteRuntime } from '@/types/site'
import { usePolling } from '@/composables/usePolling'

const route = useRoute()
const site = ref<Site | null>(null)
const runtime = ref<SiteRuntime | null>(null)
const logs = ref<string[]>([])
const activeTab = ref<'overview' | 'deploy'>('overview')

async function fetchAll() {
  const id = route.params.id as string
  site.value = await sitesApi.get(id)
  runtime.value = await sitesApi.runtime(id)
  logs.value = await sitesApi.logs(id, 100)
}

const { start: startPolling } = usePolling(fetchAll, 15000)

onMounted(async () => {
  await fetchAll()
  startPolling()
})
</script>

<template>
  <div class="space-y-6">
    <div v-if="site">
      <h2 class="text-2xl font-semibold tracking-tight">{{ site.name }}</h2>
      <p class="text-sm text-muted-foreground">站点详情 · {{ site.environment }}</p>
    </div>
    <div class="flex gap-2 border-b border-border">
      <button
        class="px-4 py-2 text-sm font-medium transition-colors border-b-2"
        :class="activeTab === 'overview' ? 'border-primary text-foreground' : 'border-transparent text-muted-foreground hover:text-foreground'"
        @click="activeTab = 'overview'"
      >运行概览</button>
      <button
        class="px-4 py-2 text-sm font-medium transition-colors border-b-2"
        :class="activeTab === 'deploy' ? 'border-primary text-foreground' : 'border-transparent text-muted-foreground hover:text-foreground'"
        @click="activeTab = 'deploy'"
      >部署与容量</button>
    </div>
    <div v-if="activeTab === 'overview'">
      <!-- Phase 4: SiteDetailOverview -->
      <div class="grid gap-4 md:grid-cols-2 lg:grid-cols-4">
        <div class="rounded-lg border border-border bg-card p-4">
          <div class="text-sm text-muted-foreground">状态</div>
          <div class="mt-1 text-lg font-semibold">{{ site?.status ?? '-' }}</div>
        </div>
        <div class="rounded-lg border border-border bg-card p-4">
          <div class="text-sm text-muted-foreground">端口</div>
          <div class="mt-1 text-lg font-semibold">{{ site?.port ?? '-' }}</div>
        </div>
        <div class="rounded-lg border border-border bg-card p-4">
          <div class="text-sm text-muted-foreground">内存</div>
          <div class="mt-1 text-lg font-semibold">{{ runtime?.memory_mb ? `${runtime.memory_mb} MB` : '-' }}</div>
        </div>
        <div class="rounded-lg border border-border bg-card p-4">
          <div class="text-sm text-muted-foreground">CPU</div>
          <div class="mt-1 text-lg font-semibold">{{ runtime?.cpu_percent != null ? `${runtime.cpu_percent}%` : '-' }}</div>
        </div>
      </div>
      <div v-if="logs.length" class="mt-4 rounded-lg border border-border bg-card p-4">
        <h3 class="mb-2 text-sm font-medium">最近日志</h3>
        <div class="max-h-64 overflow-auto rounded bg-muted p-3 font-mono text-xs leading-relaxed">
          <div v-for="(line, i) in logs" :key="i">{{ line }}</div>
        </div>
      </div>
    </div>
    <div v-else>
      <div class="rounded-lg border border-border bg-card p-8 text-center text-muted-foreground">
        部署与容量（Phase 4 P2 优先级，待实现）
      </div>
    </div>
  </div>
</template>
