<script setup lang="ts">
import { computed } from 'vue'
import { RouterLink } from 'vue-router'
import { Activity, ArrowLeft, Play, RefreshCw, Save, Square, Trash2 } from 'lucide-vue-next'
import { formatDateTime, getRuntimeLabel, getRuntimeTone } from '@/lib/collaboration'
import type {
  CollaborationEnv,
  CollaborationEffectiveStateSummary,
  CollaborationRuntimeConfig,
  CollaborationRuntimeStatus,
  CollaborationTone,
} from '@/types/collaboration'

const props = defineProps<{
  env: CollaborationEnv | null
  runtimeStatus: CollaborationRuntimeStatus | null
  runtimeConfig: CollaborationRuntimeConfig | null
  refreshing: boolean
  applying: boolean
  activating: boolean
  stopping: boolean
  deleting: boolean
  diagnosing: boolean
  error: string
  detailError: string
  effectiveState: CollaborationEffectiveStateSummary
}>()

const emit = defineEmits<{
  refresh: []
  diagnose: []
  apply: []
  activate: []
  stop: []
  delete: []
}>()

const runtimeLabel = computed(() => {
  if (!props.env) return '未选择'
  return getRuntimeLabel(props.env.id, props.runtimeStatus)
})

const runtimeTone = computed(() => getRuntimeTone(runtimeLabel.value))

const isSelectedEnvActive = computed(() => {
  if (!props.env || !props.runtimeStatus?.active) return false
  return props.runtimeStatus.env_id === props.env.id
})

const locationDbText = computed(() => {
  if (props.env?.location_dbs) return props.env.location_dbs
  if (isSelectedEnvActive.value && props.runtimeConfig?.location_dbs?.length) {
    return props.runtimeConfig.location_dbs.join(', ')
  }
  return '-'
})

function toneClass(tone: CollaborationTone) {
  switch (tone) {
    case 'success':
      return 'bg-emerald-500/10 text-emerald-600 border-emerald-500/20'
    case 'warning':
      return 'bg-amber-500/10 text-amber-600 border-amber-500/20'
    case 'danger':
      return 'bg-rose-500/10 text-rose-600 border-rose-500/20'
    default:
      return 'bg-muted text-muted-foreground border-border'
  }
}
</script>

<template>
  <section class="rounded-xl border border-border bg-card">
    <div class="flex flex-col gap-4 border-b border-border px-6 py-5 lg:flex-row lg:items-start lg:justify-between">
      <div class="space-y-3">
        <RouterLink
          to="/sites"
          class="inline-flex items-center gap-2 text-sm text-muted-foreground transition-colors hover:text-foreground"
        >
          <ArrowLeft class="h-4 w-4" />
          返回站点管理
        </RouterLink>

        <div v-if="env">
          <div class="flex flex-wrap items-center gap-2">
            <h1 class="text-2xl font-semibold tracking-tight">{{ env.name }}</h1>
            <span
              class="inline-flex items-center rounded-full border px-2.5 py-0.5 text-xs font-medium"
              :class="toneClass(runtimeTone)"
            >
              {{ runtimeLabel }}
            </span>
            <span
              v-if="runtimeStatus?.active && runtimeStatus.env_id === env.id"
              class="inline-flex items-center rounded-full border border-primary/20 bg-primary/10 px-2.5 py-0.5 text-xs font-medium text-primary"
            >
              当前激活环境
            </span>
          </div>
          <p class="mt-1 text-sm text-muted-foreground">
            {{ env.location || '未配置区域说明' }}
          </p>
        </div>

        <div v-else>
          <h1 class="text-2xl font-semibold tracking-tight">协同组详情</h1>
          <p class="mt-1 text-sm text-muted-foreground">请选择左侧协同组查看详情。</p>
        </div>
      </div>

      <div class="flex flex-wrap items-center gap-2">
        <button
          class="inline-flex h-9 items-center gap-2 rounded-md border border-input bg-background px-4 text-sm font-medium transition-colors hover:bg-accent disabled:cursor-not-allowed disabled:opacity-50"
          :disabled="!env || refreshing || applying || activating || stopping || deleting || diagnosing"
          @click="emit('refresh')"
        >
          <RefreshCw class="h-4 w-4" />
          {{ refreshing ? '刷新中' : '刷新' }}
        </button>
        <button
          class="inline-flex h-9 items-center gap-2 rounded-md border border-input bg-background px-4 text-sm font-medium transition-colors hover:bg-accent disabled:cursor-not-allowed disabled:opacity-50"
          :disabled="!env || refreshing || applying || activating || stopping || deleting || diagnosing"
          @click="emit('diagnose')"
        >
          <Activity class="h-4 w-4" />
          {{ diagnosing ? '诊断中' : '诊断' }}
        </button>
        <button
          class="inline-flex h-9 items-center gap-2 rounded-md border border-input bg-background px-4 text-sm font-medium transition-colors hover:bg-accent disabled:cursor-not-allowed disabled:opacity-50"
          :disabled="!env || refreshing || applying || activating || stopping || deleting || diagnosing"
          @click="emit('apply')"
        >
          <Save class="h-4 w-4" />
          {{ applying ? '应用中' : '应用配置' }}
        </button>
        <button
          class="inline-flex h-9 items-center gap-2 rounded-md bg-primary px-4 text-sm font-medium text-primary-foreground transition-colors hover:bg-primary/90 disabled:cursor-not-allowed disabled:opacity-50"
          :disabled="!env || refreshing || applying || activating || stopping || deleting || diagnosing"
          @click="emit('activate')"
        >
          <Play class="h-4 w-4" />
          {{ activating ? '同步中' : '同步' }}
        </button>
        <button
          class="inline-flex h-9 items-center gap-2 rounded-md border border-input bg-background px-4 text-sm font-medium transition-colors hover:bg-accent disabled:cursor-not-allowed disabled:opacity-50"
          :disabled="!env || refreshing || applying || activating || stopping || deleting || diagnosing"
          @click="emit('stop')"
        >
          <Square class="h-4 w-4" />
          {{ stopping ? '停止中' : '停止运行时' }}
        </button>
        <button
          class="inline-flex h-9 items-center gap-2 rounded-md border border-rose-500/20 bg-rose-500/5 px-4 text-sm font-medium text-rose-600 transition-colors hover:bg-rose-500/10 disabled:cursor-not-allowed disabled:opacity-50"
          :disabled="!env || refreshing || applying || activating || stopping || deleting || diagnosing"
          @click="emit('delete')"
        >
          <Trash2 class="h-4 w-4" />
          {{ deleting ? '删除中' : '删除' }}
        </button>
      </div>
    </div>

    <div v-if="error || detailError" class="space-y-3 border-b border-border px-6 py-4">
      <div
        v-if="error"
        class="rounded-lg border border-rose-500/20 bg-rose-500/5 px-4 py-3 text-sm text-rose-600"
      >
        {{ error }}
      </div>
      <div
        v-if="detailError"
        class="rounded-lg border border-amber-500/20 bg-amber-500/5 px-4 py-3 text-sm text-amber-700"
      >
        {{ detailError }}
      </div>
    </div>

    <div v-if="env" class="border-b border-border px-6 py-4">
      <div class="rounded-xl border border-border bg-background p-4">
        <div class="flex flex-wrap items-center gap-2">
          <div class="text-sm font-medium text-foreground">当前生效状态</div>
          <span
            class="inline-flex items-center rounded-full border px-2.5 py-0.5 text-xs font-medium"
            :class="toneClass(effectiveState.tone)"
          >
            {{ effectiveState.label }}
          </span>
        </div>
        <div class="mt-4 grid gap-3 md:grid-cols-2 xl:grid-cols-4">
          <div class="rounded-lg border border-border bg-card px-4 py-3">
            <div class="text-xs font-medium uppercase tracking-[0.18em] text-muted-foreground">当前运行环境</div>
            <div class="mt-2 text-sm font-medium text-foreground">{{ effectiveState.runtimeEnvName }}</div>
            <div class="mt-1 text-xs text-muted-foreground">{{ effectiveState.runtimeEnvDetail }}</div>
          </div>
          <div class="rounded-lg border border-border bg-card px-4 py-3">
            <div class="text-xs font-medium uppercase tracking-[0.18em] text-muted-foreground">当前配置来源</div>
            <div class="mt-2 text-sm font-medium text-foreground">{{ effectiveState.configSource }}</div>
            <div class="mt-1 text-xs text-muted-foreground">{{ effectiveState.configSourceDetail }}</div>
          </div>
          <div class="rounded-lg border border-border bg-card px-4 py-3">
            <div class="text-xs font-medium uppercase tracking-[0.18em] text-muted-foreground">选中协同组关系</div>
            <div class="mt-2 text-sm font-medium text-foreground">{{ effectiveState.label }}</div>
            <div class="mt-1 text-xs text-muted-foreground">{{ effectiveState.relationDetail }}</div>
          </div>
          <div class="rounded-lg border border-border bg-card px-4 py-3">
            <div class="text-xs font-medium uppercase tracking-[0.18em] text-muted-foreground">最近一次运行控制</div>
            <div
              class="mt-2 text-sm font-medium"
              :class="effectiveState.lastAction.status === 'failed' ? 'text-rose-600' : effectiveState.lastAction.status === 'success' ? 'text-emerald-600' : 'text-foreground'"
            >
              {{ effectiveState.lastAction.message }}
            </div>
            <div class="mt-1 text-xs text-muted-foreground">
              {{ formatDateTime(effectiveState.lastAction.at) }}
            </div>
          </div>
        </div>
      </div>
    </div>

    <div v-if="env" class="grid gap-4 px-6 py-5 md:grid-cols-2 xl:grid-cols-4">
      <div class="rounded-xl border border-border bg-background px-4 py-3">
        <div class="text-xs font-medium uppercase tracking-[0.18em] text-muted-foreground">MQTT</div>
        <div class="mt-2 text-sm font-medium text-foreground">
          {{ env.mqtt_host || (isSelectedEnvActive ? runtimeConfig?.mqtt_host : null) || '-' }}
        </div>
        <div class="mt-1 text-xs text-muted-foreground">
          端口 {{ env.mqtt_port || (isSelectedEnvActive ? runtimeConfig?.mqtt_port : null) || 1883 }}
        </div>
      </div>

      <div class="rounded-xl border border-border bg-background px-4 py-3">
        <div class="text-xs font-medium uppercase tracking-[0.18em] text-muted-foreground">文件服务</div>
        <div class="mt-2 text-sm font-medium text-foreground">
          {{ env.file_server_host || (isSelectedEnvActive ? runtimeConfig?.file_server_host : null) || '-' }}
        </div>
        <div class="mt-1 text-xs text-muted-foreground">
          位置 {{ env.location || (isSelectedEnvActive ? runtimeConfig?.location : null) || '-' }}
        </div>
      </div>

      <div class="rounded-xl border border-border bg-background px-4 py-3">
        <div class="text-xs font-medium uppercase tracking-[0.18em] text-muted-foreground">区域 DB</div>
        <div class="mt-2 text-sm font-medium text-foreground">{{ locationDbText }}</div>
        <div class="mt-1 text-xs text-muted-foreground">
          sync_live
          {{ isSelectedEnvActive ? (runtimeConfig?.sync_live ? '开启' : '关闭') : '未激活' }}
        </div>
      </div>

      <div class="rounded-xl border border-border bg-background px-4 py-3">
        <div class="text-xs font-medium uppercase tracking-[0.18em] text-muted-foreground">更新时间</div>
        <div class="mt-2 text-sm font-medium text-foreground">{{ formatDateTime(env.updated_at) }}</div>
        <div class="mt-1 text-xs text-muted-foreground">
          创建于 {{ formatDateTime(env.created_at) }}
        </div>
      </div>
    </div>
  </section>
</template>
