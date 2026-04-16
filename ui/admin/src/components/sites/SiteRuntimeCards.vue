<script setup lang="ts">
import type { ManagedProjectSite, ManagedSiteRuntimeStatus } from '@/types/site'

defineProps<{
  site: ManagedProjectSite | null
  runtime: ManagedSiteRuntimeStatus | null
}>()
</script>

<template>
  <div class="grid gap-4 md:grid-cols-2 lg:grid-cols-4">
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
    </div>
    <div class="rounded-lg border border-border bg-card p-4">
      <div class="text-sm text-muted-foreground">Web 服务</div>
      <div class="mt-1 text-lg font-semibold" :class="runtime?.web_running ? 'text-green-600' : 'text-muted-foreground'">
        {{ runtime?.web_running ? '运行中' : '未启动' }}
      </div>
      <div class="text-xs text-muted-foreground mt-1">PID: {{ runtime?.web_pid ?? '-' }} · 端口: {{ site?.web_port }}</div>
    </div>
    <div class="rounded-lg border border-border bg-card p-4">
      <div class="text-sm text-muted-foreground">解析状态</div>
      <div class="mt-1 text-lg font-semibold" :class="runtime?.parse_running ? 'text-blue-600' : 'text-muted-foreground'">
        {{ site?.parse_status ?? '-' }}
      </div>
      <div v-if="runtime?.parse_running" class="text-xs text-blue-600 mt-1">解析进行中...</div>
    </div>
  </div>
</template>
