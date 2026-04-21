<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useCollaborationStream } from '@/composables/useCollaborationStream'
import CollaborationActiveTasks from '@/components/collaboration/CollaborationActiveTasks.vue'
import CollaborationConfigDrawer from '@/components/collaboration/CollaborationConfigDrawer.vue'
import CollaborationEnvDrawer from '@/components/collaboration/CollaborationEnvDrawer.vue'
import CollaborationFailedTasks from '@/components/collaboration/CollaborationFailedTasks.vue'
import CollaborationSiteDrawer from '@/components/collaboration/CollaborationSiteDrawer.vue'
import CollaborationTopologyPanel from '@/components/collaboration/CollaborationTopologyPanel.vue'
import GroupDetailHeader from '@/components/collaboration/GroupDetailHeader.vue'
import GroupInsightsPanel from '@/components/collaboration/GroupInsightsPanel.vue'
import GroupListPane from '@/components/collaboration/GroupListPane.vue'
import GroupLogsPanel from '@/components/collaboration/GroupLogsPanel.vue'
import GroupOverviewPanel from '@/components/collaboration/GroupOverviewPanel.vue'
import GroupSitesPanel from '@/components/collaboration/GroupSitesPanel.vue'
import { useCollaborationStore } from '@/stores/collaboration'
import type {
  CollaborationConfig,
  CollaborationEnv,
  CollaborationSite,
  CreateCollaborationEnvRequest,
  CreateCollaborationSiteRequest,
} from '@/types/collaboration'

type WorkbenchTab = 'topo' | 'sites' | 'insight' | 'logs'
const VALID_TABS: WorkbenchTab[] = ['topo', 'sites', 'insight', 'logs']

const route = useRoute()
const router = useRouter()
const collaboration = useCollaborationStore()

const routeEnvId = computed(() => {
  const value = route.query.env
  return typeof value === 'string' ? value : null
})

function readHashTab(hash: string): WorkbenchTab {
  const raw = (hash || '').replace(/^#/, '')
  return (VALID_TABS as string[]).includes(raw) ? (raw as WorkbenchTab) : 'topo'
}

const activeTab = ref<WorkbenchTab>(readHashTab(typeof location !== 'undefined' ? location.hash : ''))

function pickTab(k: WorkbenchTab) {
  activeTab.value = k
  if (typeof location !== 'undefined' && location.hash !== `#${k}`) {
    location.hash = k
  }
}

if (typeof window !== 'undefined') {
  window.addEventListener('hashchange', () => {
    activeTab.value = readHashTab(location.hash)
  })
}

const envDrawerOpen = ref(false)
const siteDrawerOpen = ref(false)
const configDrawerOpen = ref(false)
const editingEnv = ref<CollaborationEnv | null>(null)
const editingSite = ref<CollaborationSite | null>(null)

const envActionDisabled = computed(() => {
  return collaboration.loading
    || collaboration.refreshing
    || collaboration.activating
    || collaboration.deleting
    || collaboration.diagnosing
    || collaboration.importing
    || collaboration.applying
    || collaboration.stopping
})

const siteActionDisabled = computed(() => {
  return collaboration.detailLoading
    || collaboration.refreshing
    || collaboration.activating
    || collaboration.deleting
    || collaboration.diagnosing
    || collaboration.applying
    || collaboration.stopping
})

const TAB_KEYS: Record<string, WorkbenchTab> = { '1': 'topo', '2': 'sites', '3': 'insight', '4': 'logs' }

function onGlobalKeydown(e: KeyboardEvent) {
  const tag = (e.target as HTMLElement)?.tagName
  if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') return
  if (e.key in TAB_KEYS) {
    e.preventDefault()
    pickTab(TAB_KEYS[e.key])
  }
}

onMounted(async () => {
  document.addEventListener('keydown', onGlobalKeydown)
  await collaboration.initialize(routeEnvId.value)
})

onUnmounted(() => {
  document.removeEventListener('keydown', onGlobalKeydown)
})

watch(routeEnvId, async (nextEnvId) => {
  if (!nextEnvId || nextEnvId === collaboration.selectedEnvId) return
  if (!collaboration.envs.length) return
  await collaboration.selectEnv(nextEnvId)
})

watch(
  () => collaboration.selectedEnvId,
  (nextEnvId) => {
    if (routeEnvId.value === nextEnvId) return
    const query = { ...route.query }
    if (nextEnvId) {
      query.env = nextEnvId
    } else {
      delete query.env
    }
    router.replace({ query })
  },
)

async function handleDelete() {
  if (!collaboration.selectedEnv) return
  const confirmed = window.confirm(`确定删除协同组“${collaboration.selectedEnv.name}”吗？`)
  if (!confirmed) return
  await collaboration.deleteSelectedEnv()
}

function openCreateEnvDrawer() {
  editingEnv.value = null
  envDrawerOpen.value = true
}

function openEditEnvDrawer(envId: string) {
  editingEnv.value = collaboration.envs.find((item) => item.id === envId) ?? null
  if (!editingEnv.value) return
  envDrawerOpen.value = true
}

async function saveEnv(payload: CreateCollaborationEnvRequest) {
  if (editingEnv.value) {
    await collaboration.updateEnv(editingEnv.value.id, payload)
  } else {
    await collaboration.createEnv(payload)
  }
}

function closeEnvDrawer() {
  envDrawerOpen.value = false
  editingEnv.value = null
}

function openCreateSiteDrawer() {
  if (!collaboration.selectedEnv) return
  editingSite.value = null
  siteDrawerOpen.value = true
}

function openEditSiteDrawer(siteId: string) {
  editingSite.value = collaboration.sites.find((item) => item.id === siteId) ?? null
  if (!editingSite.value) return
  siteDrawerOpen.value = true
}

async function saveSite(payload: CreateCollaborationSiteRequest) {
  if (editingSite.value) {
    await collaboration.updateSite(editingSite.value.id, payload)
  } else {
    await collaboration.createSite(payload)
  }
}

function closeSiteDrawer() {
  siteDrawerOpen.value = false
  editingSite.value = null
}

async function handleDeleteSite(siteId: string) {
  const target = collaboration.sites.find((item) => item.id === siteId)
  if (!target) return
  const confirmed = window.confirm(`确定删除协同站点“${target.name}”吗？`)
  if (!confirmed) return
  await collaboration.deleteSite(siteId)
}

async function handleImportCurrentConfig() {
  await collaboration.importEnvFromDbOption()
}

async function saveCollabConfigLocal(next: CollaborationConfig) {
  await collaboration.saveCollabConfig(next)
}

// ─────────────── ROADMAP · M3 · 实时通道接入 ───────────────
// Phase 9 后端就绪前：DEV 模式或 ?mock=sse 时走本地 mock；
// 生产模式下会尝试连接 `/api/remote-sync/events/stream`，后端未就绪会静默重连。
const stream = useCollaborationStream({
  config: {
    reconnect_initial_ms: collaboration.collabConfig.reconnect_initial_ms,
    reconnect_max_ms: collaboration.collabConfig.reconnect_max_ms,
  },
  callbacks: {
    onActiveTask(task) {
      const prev = collaboration.activeTasks
      const idx = prev.findIndex((t) => t.task_id === task.task_id)
      if (idx >= 0) {
        collaboration.activeTasks = prev.map((t, i) => (i === idx ? task : t))
      } else {
        collaboration.activeTasks = [task, ...prev].slice(0, 12)
      }
      if (task.status === 'Completed') {
        collaboration.activeTasks = collaboration.activeTasks.filter((t) => t.task_id !== task.task_id)
      }
    },
    onFailedTask(task) {
      const prev = collaboration.failedTasks
      if (!prev.some((t) => t.id === task.id)) {
        collaboration.failedTasks = [task, ...prev].slice(0, 50)
      }
    },
    onSyncCompleted(evt) {
      collaboration.pushToast({
        type: 'success',
        icon: '✓',
        title: '同步完成',
        message: evt.message ?? `${evt.site_id} · ${evt.file_count} 个文件`,
        durationMs: 6000,
      })
    },
    onSyncFailed(evt) {
      collaboration.pushToast({
        type: 'error',
        icon: '!',
        title: '同步失败',
        message: `${evt.site_id} · ${evt.error}`,
        durationMs: 8000,
      })
    },
  },
})

watch(
  () => stream.realtimeConnected.value,
  (v) => {
    collaboration.realtimeConnected = v
  },
  { immediate: true },
)
</script>

<template>
  <div class="space-y-6">
    <div class="flex flex-wrap items-start justify-between gap-4">
      <div>
        <h1 class="text-3xl font-semibold tracking-tight">异地协同</h1>
        <p class="mt-2 text-sm text-muted-foreground">
          左侧选择协同组，右侧通过四个 Tab 查看拓扑、站点、洞察与实时日志；支持参数配置与实时推送。
        </p>
      </div>
      <div class="flex items-center gap-3 text-xs">
        <span
          class="inline-flex items-center gap-2 rounded-full border px-3 py-1.5 font-medium"
          :class="collaboration.realtimeConnected ? 'border-emerald-500/30 bg-emerald-500/10 text-emerald-600' : 'border-muted bg-muted/30 text-muted-foreground'"
        >
          <span
            class="relative flex h-2 w-2"
          >
            <span
              v-if="collaboration.realtimeConnected"
              class="absolute inline-flex h-full w-full animate-ping rounded-full bg-emerald-400 opacity-75"
            />
            <span
              class="relative inline-flex h-2 w-2 rounded-full"
              :class="collaboration.realtimeConnected ? 'bg-emerald-500' : 'bg-muted-foreground/60'"
            />
          </span>
          {{ collaboration.realtimeConnected ? 'ONLINE' : '轮询模式' }}
        </span>
        <button
          class="inline-flex h-9 items-center gap-2 rounded-md border border-input bg-background px-3 text-sm font-medium transition-colors hover:bg-accent disabled:pointer-events-none disabled:opacity-50"
          :disabled="!collaboration.selectedEnv || envActionDisabled"
          @click="configDrawerOpen = true"
        >
          <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="12" cy="12" r="3" />
            <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z" />
          </svg>
          参数配置
        </button>
      </div>
    </div>

    <div class="grid gap-6 xl:grid-cols-[320px_minmax(0,1fr)]">
      <GroupListPane
        :items="collaboration.groups"
        :selected-id="collaboration.selectedEnvId"
        :loading="collaboration.loading"
        :error="collaboration.error"
        :action-disabled="envActionDisabled"
        @retry="collaboration.initialize(routeEnvId)"
        @create="openCreateEnvDrawer"
        @import="handleImportCurrentConfig"
        @edit="openEditEnvDrawer"
        @select="collaboration.selectEnv"
      />

      <div class="space-y-6 min-w-0">
        <GroupDetailHeader
          :env="collaboration.selectedEnv"
          :runtime-status="collaboration.runtimeStatus"
          :runtime-config="collaboration.runtimeConfig"
          :refreshing="collaboration.refreshing"
          :applying="collaboration.applying"
          :activating="collaboration.activating"
          :stopping="collaboration.stopping"
          :deleting="collaboration.deleting"
          :diagnosing="collaboration.diagnosing"
          :error="collaboration.error"
          :detail-error="collaboration.detailError"
          :effective-state="collaboration.effectiveState"
          @refresh="collaboration.refreshAll"
          @diagnose="collaboration.runDiagnostics"
          @apply="collaboration.applySelectedEnv"
          @activate="collaboration.activateSelectedEnv"
          @stop="collaboration.stopCurrentRuntime"
          @delete="handleDelete"
        />

        <template v-if="collaboration.selectedEnv">
          <GroupOverviewPanel
            :metrics="collaboration.overviewMetrics"
            :loading="collaboration.detailLoading"
          />

          <!-- Tabs shell -->
          <div class="collab-tabs">
            <button
              v-for="tab in ([
                { k: 'topo', label: '拓扑', cnt: collaboration.siteCards.length || null },
                { k: 'sites', label: '站点', cnt: collaboration.siteCards.length || null },
                { k: 'insight', label: '洞察', cnt: null },
                { k: 'logs', label: '日志', cnt: collaboration.logsTotal || null },
              ] as Array<{ k: WorkbenchTab; label: string; cnt: number | null }>)"
              :key="tab.k"
              :class="['collab-tab', { active: activeTab === tab.k }]"
              @click="pickTab(tab.k)"
            >
              {{ tab.label }}
              <span v-if="tab.cnt != null" class="cnt">{{ tab.cnt }}</span>
            </button>
          </div>

          <!-- Tab 1: 拓扑 -->
          <template v-if="activeTab === 'topo'">
            <CollaborationTopologyPanel
              :env="collaboration.selectedEnv"
              :items="collaboration.siteCards"
              :flows="collaboration.insightsSummary.topFailedFlows"
              :loading="collaboration.detailLoading"
              :action-disabled="siteActionDisabled"
              @diagnose="collaboration.runSiteDiagnostic"
              @refresh="collaboration.refreshAll"
              @select="openEditSiteDrawer"
            />
          </template>

          <!-- Tab 2: 站点 -->
          <template v-if="activeTab === 'sites'">
            <GroupSitesPanel
              :items="collaboration.siteCards"
              :loading="collaboration.detailLoading"
              :error="collaboration.detailError"
              :action-disabled="siteActionDisabled"
              @create="openCreateSiteDrawer"
              @edit="openEditSiteDrawer"
              @delete="handleDeleteSite"
              @diagnose="collaboration.runSiteDiagnostic"
            />
          </template>

          <!-- Tab 3: 洞察 -->
          <template v-if="activeTab === 'insight'">
            <GroupInsightsPanel
              :summary="collaboration.insightsSummary"
              :loading="collaboration.detailLoading"
              :error="collaboration.detailError"
            />
            <div v-if="collaboration.failedTasks.length" class="rounded-xl border border-border bg-card p-5 mt-6">
              <CollaborationFailedTasks
                :items="collaboration.failedTasks"
                :loading="collaboration.detailLoading"
                @retry="collaboration.retryFailedTask"
                @cleanup="collaboration.cleanupExhaustedFailedTasks"
                @inspect="(id) => void id"
              />
            </div>
          </template>

          <!-- Tab 4: 日志 -->
          <template v-if="activeTab === 'logs'">
            <div v-if="collaboration.activeTasks.length" class="rounded-xl border border-border bg-card p-5">
              <CollaborationActiveTasks
                :items="collaboration.activeTasks"
                :loading="collaboration.detailLoading"
                title="进行中任务"
              />
            </div>

            <GroupLogsPanel
              :logs="collaboration.logs"
              :total="collaboration.logsTotal"
              :loading="collaboration.detailLoading || collaboration.logsLoading"
              :error="collaboration.detailError"
              :filters="collaboration.logFilters"
              :target-site-options="collaboration.logTargetOptions"
              :direction-options="collaboration.logDirectionOptions"
              :status-options="collaboration.logStatusOptions"
              @update-filters="collaboration.setLogFilters"
            />
          </template>
        </template>

        <div
          v-else
          class="rounded-xl border border-dashed border-border bg-muted/20 px-6 py-12 text-center text-sm text-muted-foreground"
        >
          当前没有可查看的协同组。
        </div>
      </div>
    </div>

    <CollaborationEnvDrawer
      :open="envDrawerOpen"
      :env="editingEnv"
      :disabled="envActionDisabled"
      :save="saveEnv"
      @close="closeEnvDrawer"
    />

    <CollaborationSiteDrawer
      :open="siteDrawerOpen"
      :site="editingSite"
      :disabled="siteActionDisabled"
      :save="saveSite"
      @close="closeSiteDrawer"
    />

    <CollaborationConfigDrawer
      :open="configDrawerOpen"
      :config="collaboration.collabConfig"
      :env-label="collaboration.selectedEnv?.name ?? null"
      :disabled="collaboration.collabConfigSaving"
      :save="saveCollabConfigLocal"
      @close="configDrawerOpen = false"
    />
  </div>
</template>

<style scoped>
.collab-tabs {
  display: flex;
  align-items: flex-end;
  gap: 28px;
  border-bottom: 1px solid hsl(var(--border));
  padding: 0 4px;
}
.collab-tab {
  position: relative;
  padding: 9px 0 11px;
  font-size: 13.5px;
  color: hsl(var(--muted-foreground));
  cursor: pointer;
  font-weight: 500;
  letter-spacing: 0.01em;
  border: 0;
  background: transparent;
  transition: color 0.15s;
}
.collab-tab:hover {
  color: hsl(var(--foreground));
}
.collab-tab.active {
  color: hsl(var(--foreground));
}
.collab-tab.active::after {
  content: "";
  position: absolute;
  left: 0;
  right: 0;
  bottom: -1px;
  height: 2px;
  background: var(--collab-brand, hsl(var(--primary)));
  border-radius: 2px 2px 0 0;
}
.collab-tab .cnt {
  margin-left: 6px;
  font-size: 11.5px;
  color: hsl(var(--muted-foreground));
  font-family: var(--collab-font-mono);
  font-variant-numeric: tabular-nums;
}
</style>
