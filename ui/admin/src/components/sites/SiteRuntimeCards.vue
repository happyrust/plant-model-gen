<script setup lang="ts">
import type { ManagedProjectSite, ManagedSiteRuntimeStatus } from '@/types/site'

defineProps<{
  site: ManagedProjectSite | null
  runtime: ManagedSiteRuntimeStatus | null
}>()

function businessLabel(value?: boolean | null) {
  if (value === true) return '通过'
  if (value === false) return '失败'
  return '未检查'
}

function businessClass(value?: boolean | null) {
  if (value === true) return 'text-green-600'
  if (value === false) return 'text-red-600'
  return 'text-muted-foreground'
}
</script>

<template>
  <div class="grid gap-4 md:grid-cols-2 lg:grid-cols-5">
    <div class="rounded-lg border border-border bg-card p-4">
      <div class="text-sm text-muted-foreground">当前阶段</div>
      <div class="mt-1 text-lg font-semibold">{{ runtime?.current_stage_label ?? site?.status ?? '-' }}</div>
      <div v-if="runtime?.current_stage_detail" class="text-xs text-muted-foreground mt-1">{{ runtime.current_stage_detail }}</div>
    </div>
    <div class="rounded-lg border border-border bg-card p-4">
      <div class="text-sm text-muted-foreground">数据库</div>
      <div class="mt-1 text-lg font-semibold" :class="runtime?.db_running ? 'text-green-600' : 'text-muted-foreground'">
        {{ runtime?.db_running ? '运行中' : '未启动' }}
      </div>
      <div class="text-xs text-muted-foreground mt-1">PID: {{ runtime?.db_pid ?? '-' }} · 端口: {{ site?.db_port }}</div>
      <div class="mt-2 space-y-1 text-xs">
        <div :class="businessClass(runtime?.database_connected)">业务 DB：{{ businessLabel(runtime?.database_connected) }}</div>
        <div :class="businessClass(runtime?.surrealdb_connected)">Surreal：{{ businessLabel(runtime?.surrealdb_connected) }}</div>
      </div>
    </div>
    <div class="rounded-lg border border-border bg-card p-4">
      <div class="text-sm text-muted-foreground">Web 服务</div>
      <div class="mt-1 text-lg font-semibold" :class="runtime?.web_running ? 'text-green-600' : 'text-muted-foreground'">
        {{ runtime?.web_running ? '运行中' : '未启动' }}
      </div>
      <div class="text-xs text-muted-foreground mt-1">PID: {{ runtime?.web_pid ?? '-' }} · 端口: {{ site?.web_port }}</div>
      <div class="mt-2 space-y-1 text-xs">
        <div :class="businessClass(runtime?.web_status_ok)">/api/status：{{ businessLabel(runtime?.web_status_ok) }}</div>
        <div :class="businessClass(runtime?.site_identity_ok)">站点身份：{{ businessLabel(runtime?.site_identity_ok) }}</div>
      </div>
    </div>
    <div class="rounded-lg border border-border bg-card p-4">
      <div class="text-sm text-muted-foreground">Viewer</div>
      <div class="mt-1 text-lg font-semibold" :class="runtime?.viewer_running ? 'text-green-600' : 'text-muted-foreground'">
        {{ runtime?.viewer_running ? '运行中' : '未启动' }}
      </div>
      <div class="text-xs text-muted-foreground mt-1">PID: {{ runtime?.viewer_pid ?? '-' }} · 端口: {{ runtime?.viewer_port ?? site?.viewer_port ?? '-' }}</div>
    </div>
    <div class="rounded-lg border border-border bg-card p-4">
      <div class="text-sm text-muted-foreground">解析状态</div>
      <div class="mt-1 text-lg font-semibold" :class="runtime?.parse_running ? 'text-blue-600' : 'text-muted-foreground'">
        {{ site?.parse_status ?? '-' }}
      </div>
      <div v-if="runtime?.parse_running" class="text-xs text-blue-600 mt-1">解析进行中...</div>
      <div v-if="runtime?.sidecar_job_id" class="mt-2 space-y-1 text-xs text-muted-foreground">
        <div>Sidecar: {{ runtime.sidecar_job_kind ?? '-' }} / {{ runtime.sidecar_job_status ?? '-' }}</div>
        <div class="truncate font-mono" :title="runtime.sidecar_job_id">Job: {{ runtime.sidecar_job_id }}</div>
      </div>
    </div>
  </div>
</template>
