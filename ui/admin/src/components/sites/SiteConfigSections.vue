<script setup lang="ts">
import type { ManagedProjectSite } from '@/types/site'

defineProps<{
  site: ManagedProjectSite
}>()

function formatTime(value?: string | null) {
  if (!value) return '-'
  const d = new Date(value)
  if (Number.isNaN(d.getTime())) return '-'
  return d.toLocaleString('zh-CN', {
    year: 'numeric', month: '2-digit', day: '2-digit',
    hour: '2-digit', minute: '2-digit', second: '2-digit',
  })
}
</script>

<template>
  <div class="space-y-4">
    <div class="rounded-lg border border-border bg-card p-5">
      <h4 class="text-sm font-medium text-muted-foreground mb-3">项目信息</h4>
      <div class="grid grid-cols-[auto_1fr] gap-x-6 gap-y-2 text-sm">
        <span class="text-muted-foreground">项目名称</span><span>{{ site.project_name }}</span>
        <span class="text-muted-foreground">项目代码</span><span>{{ site.project_code }}</span>
        <span class="text-muted-foreground">项目路径</span><span class="break-all">{{ site.project_path }}</span>
        <span class="text-muted-foreground">关联工程</span>
        <span>{{ site.associated_project || site.project_name }} <span v-if="!site.associated_project" class="text-xs text-muted-foreground">(默认)</span></span>
      </div>
    </div>

    <div class="rounded-lg border border-border bg-card p-5">
      <h4 class="text-sm font-medium text-muted-foreground mb-3">运行配置</h4>
      <div class="grid grid-cols-[auto_1fr] gap-x-6 gap-y-2 text-sm">
        <span class="text-muted-foreground">DB 端口</span><span class="font-mono">{{ site.db_port }}</span>
        <span class="text-muted-foreground">Web 端口</span><span class="font-mono">{{ site.web_port }}</span>
        <span class="text-muted-foreground">绑定地址</span><span>{{ site.bind_host || '0.0.0.0' }}</span>
        <span class="text-muted-foreground">对外访问</span><span>{{ site.public_base_url || '未配置（仅本机地址）' }}</span>
        <span class="text-muted-foreground">手动 DB Nums</span>
        <span>{{ site.manual_db_nums.length ? site.manual_db_nums.join(', ') : '自动检测' }}</span>
      </div>
    </div>

    <div class="rounded-lg border border-border bg-card p-5">
      <h4 class="text-sm font-medium text-muted-foreground mb-3">路径信息</h4>
      <div class="grid grid-cols-[auto_1fr] gap-x-6 gap-y-2 text-sm">
        <span class="text-muted-foreground">配置路径</span><span class="break-all font-mono text-xs">{{ site.config_path }}</span>
        <span class="text-muted-foreground">运行目录</span><span class="break-all font-mono text-xs">{{ site.runtime_dir }}</span>
        <span class="text-muted-foreground">数据目录</span><span class="break-all font-mono text-xs">{{ site.db_data_path }}</span>
      </div>
    </div>

    <div class="rounded-lg border border-border bg-card p-5">
      <h4 class="text-sm font-medium text-muted-foreground mb-3">时间信息</h4>
      <div class="grid grid-cols-[auto_1fr] gap-x-6 gap-y-2 text-sm">
        <span class="text-muted-foreground">创建时间</span><span>{{ formatTime(site.created_at) }}</span>
        <span class="text-muted-foreground">更新时间</span><span>{{ formatTime(site.updated_at) }}</span>
      </div>
    </div>
  </div>
</template>
