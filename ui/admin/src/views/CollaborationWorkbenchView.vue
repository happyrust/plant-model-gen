<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import CollaborationEnvDrawer from '@/components/collaboration/CollaborationEnvDrawer.vue'
import CollaborationSiteDrawer from '@/components/collaboration/CollaborationSiteDrawer.vue'
import GroupDetailHeader from '@/components/collaboration/GroupDetailHeader.vue'
import GroupInsightsPanel from '@/components/collaboration/GroupInsightsPanel.vue'
import GroupListPane from '@/components/collaboration/GroupListPane.vue'
import GroupLogsPanel from '@/components/collaboration/GroupLogsPanel.vue'
import GroupOverviewPanel from '@/components/collaboration/GroupOverviewPanel.vue'
import GroupSitesPanel from '@/components/collaboration/GroupSitesPanel.vue'
import { useCollaborationStore } from '@/stores/collaboration'
import type {
  CollaborationEnv,
  CollaborationSite,
  CreateCollaborationEnvRequest,
  CreateCollaborationSiteRequest,
} from '@/types/collaboration'

const route = useRoute()
const router = useRouter()
const collaboration = useCollaborationStore()

const routeEnvId = computed(() => {
  const value = route.query.env
  return typeof value === 'string' ? value : null
})

const envDrawerOpen = ref(false)
const siteDrawerOpen = ref(false)
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

onMounted(async () => {
  await collaboration.initialize(routeEnvId.value)
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
</script>

<template>
  <div class="space-y-6">
    <div>
      <h1 class="text-3xl font-semibold tracking-tight">异地协同</h1>
      <p class="mt-2 text-sm text-muted-foreground">
        左侧选择协同组，右侧查看协同概览、主动诊断、站点详情、同步洞察与实时日志。
      </p>
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

      <div class="space-y-6">
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

          <div class="grid gap-6 2xl:grid-cols-[1.15fr_0.85fr]">
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
            <GroupInsightsPanel
              :summary="collaboration.insightsSummary"
              :loading="collaboration.detailLoading"
              :error="collaboration.detailError"
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
  </div>
</template>
