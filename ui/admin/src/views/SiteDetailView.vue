<script setup lang="ts">
import { ref, onMounted, computed } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { sitesApi } from '@/api/sites'
import { useSitesStore } from '@/stores/sites'
import type { ManagedProjectSite, ManagedSiteRuntimeStatus, ManagedSiteLogsResponse } from '@/types/site'
import { usePolling } from '@/composables/usePolling'
import { ArrowLeft, Play, Square, RefreshCw } from 'lucide-vue-next'

const route = useRoute()
const router = useRouter()
const sitesStore = useSitesStore()

const site = ref<ManagedProjectSite | null>(null)
const runtime = ref<ManagedSiteRuntimeStatus | null>(null)
const logsData = ref<ManagedSiteLogsResponse | null>(null)
const activeTab = ref<'overview' | 'deploy'>('overview')
const activeLogTab = ref<'parse' | 'db' | 'web'>('parse')

const siteId = computed(() => route.params.id as string)

const selectedLogs = computed(() => {
  if (!logsData.value) return []
  switch (activeLogTab.value) {
    case 'parse': return logsData.value.parse_log
    case 'db': return logsData.value.db_log
    case 'web': return logsData.value.web_log
  }
})

async function fetchAll() {
  const id = siteId.value
  try {
    site.value = await sitesApi.get(id)
    runtime.value = await sitesApi.runtime(id)
    logsData.value = await sitesApi.logs(id)
  } catch {
    // partial failure is acceptable
  }
}

const { start: startPolling } = usePolling(fetchAll, 10000)

onMounted(async () => {
  await fetchAll()
  startPolling()
})
</script>

<template>
  <div class="space-y-6">
    <div class="flex items-center gap-4">
      <button @click="router.push('/sites')"
        class="inline-flex h-8 w-8 items-center justify-center rounded-md hover:bg-accent transition-colors">
        <ArrowLeft class="h-4 w-4" />
      </button>
      <div v-if="site">
        <h2 class="text-2xl font-semibold tracking-tight">{{ site.project_name }}</h2>
        <p class="text-sm text-muted-foreground">{{ site.site_id }} · {{ site.status }}</p>
      </div>
      <div v-if="site" class="ml-auto flex gap-2">
        <button v-if="['Stopped', 'Parsed', 'Failed', 'Draft'].includes(site.status)"
          @click="sitesStore.startSite(site.site_id).then(fetchAll)"
          class="inline-flex h-9 items-center gap-2 rounded-md bg-green-600 px-4 text-sm font-medium text-white shadow hover:bg-green-700 transition-colors">
          <Play class="h-4 w-4" /> 启动
        </button>
        <button v-if="site.status === 'Running'"
          @click="sitesStore.stopSite(site.site_id).then(fetchAll)"
          class="inline-flex h-9 items-center gap-2 rounded-md bg-amber-600 px-4 text-sm font-medium text-white shadow hover:bg-amber-700 transition-colors">
          <Square class="h-4 w-4" /> 停止
        </button>
        <button @click="sitesStore.parseSite(siteId).then(fetchAll)"
          :disabled="site.parse_status === 'Running'"
          class="inline-flex h-9 items-center gap-2 rounded-md border border-input bg-transparent px-4 text-sm font-medium shadow-sm hover:bg-accent transition-colors disabled:opacity-50">
          <RefreshCw class="h-4 w-4" /> 解析
        </button>
      </div>
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
      >配置信息</button>
    </div>

    <div v-if="activeTab === 'overview'" class="space-y-4">
      <div class="grid gap-4 md:grid-cols-2 lg:grid-cols-4">
        <div class="rounded-lg border border-border bg-card p-4">
          <div class="text-sm text-muted-foreground">站点状态</div>
          <div class="mt-1 text-lg font-semibold">{{ runtime?.current_stage_label ?? site?.status ?? '-' }}</div>
          <div v-if="runtime?.current_stage_detail" class="text-xs text-muted-foreground mt-1">{{ runtime.current_stage_detail }}</div>
        </div>
        <div class="rounded-lg border border-border bg-card p-4">
          <div class="text-sm text-muted-foreground">数据库</div>
          <div class="mt-1 text-lg font-semibold" :class="runtime?.db_running ? 'text-green-600' : 'text-muted-foreground'">
            {{ runtime?.db_running ? '运行中' : '未启动' }}
          </div>
          <div class="text-xs text-muted-foreground mt-1">PID: {{ runtime?.db_pid ?? '-' }} / 端口: {{ site?.db_port }}</div>
        </div>
        <div class="rounded-lg border border-border bg-card p-4">
          <div class="text-sm text-muted-foreground">Web 服务</div>
          <div class="mt-1 text-lg font-semibold" :class="runtime?.web_running ? 'text-green-600' : 'text-muted-foreground'">
            {{ runtime?.web_running ? '运行中' : '未启动' }}
          </div>
          <div class="text-xs text-muted-foreground mt-1">PID: {{ runtime?.web_pid ?? '-' }} / 端口: {{ site?.web_port }}</div>
        </div>
        <div class="rounded-lg border border-border bg-card p-4">
          <div class="text-sm text-muted-foreground">解析状态</div>
          <div class="mt-1 text-lg font-semibold" :class="runtime?.parse_running ? 'text-blue-600' : 'text-muted-foreground'">
            {{ site?.parse_status ?? '-' }}
          </div>
          <div v-if="runtime?.parse_running" class="text-xs text-blue-600 mt-1">解析进行中...</div>
        </div>
      </div>

      <div v-if="runtime?.last_error" class="rounded-lg border border-destructive/50 bg-destructive/5 p-4">
        <div class="text-sm font-medium text-destructive">最近错误</div>
        <div class="mt-1 text-sm text-destructive/80">{{ runtime.last_error }}</div>
      </div>

      <div v-if="runtime?.entry_url" class="rounded-lg border border-border bg-card p-4">
        <div class="text-sm text-muted-foreground mb-1">访问地址</div>
        <a :href="runtime.entry_url" target="_blank" class="text-sm text-primary hover:underline">{{ runtime.entry_url }}</a>
      </div>

      <!-- Logs -->
      <div class="rounded-lg border border-border bg-card">
        <div class="flex items-center gap-2 border-b border-border px-4 py-2">
          <button v-for="tab in (['parse', 'db', 'web'] as const)" :key="tab"
            @click="activeLogTab = tab"
            class="rounded-md px-3 py-1 text-xs font-medium transition-colors"
            :class="activeLogTab === tab ? 'bg-accent text-accent-foreground' : 'text-muted-foreground hover:text-foreground'">
            {{ tab === 'parse' ? '解析日志' : tab === 'db' ? 'DB 日志' : 'Web 日志' }}
          </button>
        </div>
        <div class="max-h-80 overflow-auto p-4">
          <div v-if="!selectedLogs.length" class="text-sm text-muted-foreground text-center py-4">暂无日志</div>
          <div v-else class="font-mono text-xs leading-relaxed space-y-0.5">
            <div v-for="(line, i) in selectedLogs" :key="i" class="whitespace-pre-wrap break-all">{{ line }}</div>
          </div>
        </div>
      </div>
    </div>

    <div v-else class="space-y-4">
      <div v-if="site" class="rounded-lg border border-border bg-card p-6">
        <h3 class="text-lg font-medium mb-4">站点配置</h3>
        <div class="grid grid-cols-2 gap-y-3 text-sm">
          <div class="text-muted-foreground">项目名称</div><div>{{ site.project_name }}</div>
          <div class="text-muted-foreground">项目代码</div><div>{{ site.project_code }}</div>
          <div class="text-muted-foreground">项目路径</div><div class="break-all">{{ site.project_path }}</div>
          <div class="text-muted-foreground">DB 端口</div><div>{{ site.db_port }}</div>
          <div class="text-muted-foreground">Web 端口</div><div>{{ site.web_port }}</div>
          <div class="text-muted-foreground">绑定地址</div><div>{{ site.bind_host || '0.0.0.0' }}</div>
          <div class="text-muted-foreground">手动 DB Nums</div>
          <div>{{ site.manual_db_nums.length ? site.manual_db_nums.join(', ') : '自动检测' }}</div>
          <div class="text-muted-foreground">配置路径</div><div class="break-all">{{ site.config_path }}</div>
          <div class="text-muted-foreground">运行目录</div><div class="break-all">{{ site.runtime_dir }}</div>
          <div class="text-muted-foreground">数据目录</div><div class="break-all">{{ site.db_data_path }}</div>
          <div class="text-muted-foreground">创建时间</div><div>{{ site.created_at }}</div>
          <div class="text-muted-foreground">更新时间</div><div>{{ site.updated_at }}</div>
        </div>
      </div>
    </div>
  </div>
</template>
