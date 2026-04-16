<script setup lang="ts">
import { Activity, PenLine, Plus, Trash2 } from 'lucide-vue-next'
import { formatDateTime } from '@/lib/collaboration'
import type { CollaborationSiteAvailability, CollaborationSiteCard, CollaborationTone } from '@/types/collaboration'

defineProps<{
  items: CollaborationSiteCard[]
  loading: boolean
  error: string
  actionDisabled?: boolean
}>()

const emit = defineEmits<{
  create: []
  edit: [id: string]
  delete: [id: string]
  diagnose: [id: string]
}>()

function stateClass(state: CollaborationSiteAvailability) {
  switch (state) {
    case 'online':
      return 'bg-emerald-500/10 text-emerald-600 border-emerald-500/20'
    case 'cached':
      return 'bg-amber-500/10 text-amber-600 border-amber-500/20'
    default:
      return 'bg-muted text-muted-foreground border-border'
  }
}

function diagnosticClass(tone: CollaborationTone) {
  switch (tone) {
    case 'success':
      return 'border-emerald-500/20 bg-emerald-500/5 text-emerald-600'
    case 'warning':
      return 'border-amber-500/20 bg-amber-500/5 text-amber-700'
    case 'danger':
      return 'border-rose-500/20 bg-rose-500/5 text-rose-600'
    default:
      return 'border-border bg-muted/40 text-muted-foreground'
  }
}
</script>

<template>
  <section class="rounded-xl border border-border bg-card">
    <div class="border-b border-border px-5 py-4">
      <div class="flex items-start justify-between gap-3">
        <div>
          <p class="text-xs font-medium uppercase tracking-[0.18em] text-muted-foreground">协同站点卡片</p>
          <h2 class="mt-1 text-lg font-semibold">站点状态与连接信息</h2>
          <p class="mt-1 text-sm text-muted-foreground">展示各协同站点的主动诊断、连接摘要、主从角色与元数据状态。</p>
        </div>
        <button
          class="inline-flex h-9 items-center gap-2 rounded-md bg-primary px-3 text-sm font-medium text-primary-foreground shadow transition-colors hover:bg-primary/90 disabled:pointer-events-none disabled:opacity-50"
          :disabled="actionDisabled"
          @click="emit('create')"
        >
          <Plus class="h-4 w-4" />
          新增站点
        </button>
      </div>
    </div>

    <div class="p-5">
      <div
        v-if="error"
        class="mb-4 rounded-lg border border-amber-500/20 bg-amber-500/5 px-4 py-3 text-sm text-amber-700"
      >
        {{ error }}
      </div>

      <div v-if="loading && !items.length" class="grid gap-4 md:grid-cols-2">
        <div
          v-for="index in 4"
          :key="index"
          class="h-56 animate-pulse rounded-xl border border-border bg-muted/50"
        />
      </div>

      <div
        v-else-if="!items.length"
        class="rounded-xl border border-dashed border-border bg-muted/20 px-4 py-10 text-center text-sm text-muted-foreground"
      >
        当前协同组没有登记站点。
      </div>

      <div v-else class="grid gap-4 md:grid-cols-2">
        <article
          v-for="item in items"
          :key="item.id"
          class="rounded-xl border border-border bg-background p-4"
        >
          <div class="flex items-start justify-between gap-3">
            <div class="min-w-0">
              <h3 class="truncate text-base font-semibold text-foreground">{{ item.name }}</h3>
              <p class="mt-1 text-sm text-muted-foreground">{{ item.location || '未配置站点区域' }}</p>
            </div>
            <div class="flex shrink-0 flex-wrap justify-end gap-2">
              <span
                class="inline-flex items-center rounded-full border px-2 py-0.5 text-xs font-medium"
                :class="stateClass(item.availability)"
              >
                {{ item.availabilityLabel }}
              </span>
              <span class="inline-flex items-center rounded-full border border-border bg-muted px-2 py-0.5 text-xs font-medium text-muted-foreground">
                {{ item.roleLabel }}
              </span>
              <button
                class="inline-flex h-7 items-center gap-1 rounded-md border border-input bg-background px-2 text-xs text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:pointer-events-none disabled:opacity-50"
                :disabled="actionDisabled || item.diagnosticPending"
                @click="emit('diagnose', item.id)"
              >
                <Activity class="h-3.5 w-3.5" />
                {{ item.diagnosticPending ? '测试中' : '测试站点' }}
              </button>
              <button
                class="inline-flex h-7 w-7 items-center justify-center rounded-md border border-border bg-background text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:pointer-events-none disabled:opacity-50"
                :disabled="actionDisabled"
                @click="emit('edit', item.id)"
              >
                <PenLine class="h-4 w-4" />
              </button>
              <button
                class="inline-flex h-7 w-7 items-center justify-center rounded-md border border-rose-500/20 bg-rose-500/5 text-rose-600 transition-colors hover:bg-rose-500/10 disabled:pointer-events-none disabled:opacity-50"
                :disabled="actionDisabled"
                @click="emit('delete', item.id)"
              >
                <Trash2 class="h-4 w-4" />
              </button>
            </div>
          </div>

          <div class="mt-4 rounded-lg border px-3 py-3 text-sm" :class="diagnosticClass(item.diagnosticTone)">
            <div class="flex flex-wrap items-center justify-between gap-2">
              <div class="font-medium">主动诊断 · {{ item.diagnosticStatusLabel }}</div>
              <div class="text-xs opacity-80">{{ formatDateTime(item.diagnosticCheckedAt) }}</div>
            </div>
            <div class="mt-2 text-xs opacity-90">
              {{ item.diagnosticMessage || '尚未执行站点诊断' }}
            </div>
            <div class="mt-2 flex flex-wrap gap-x-4 gap-y-1 text-xs opacity-80">
              <span v-if="item.diagnosticUrl">目标 {{ item.diagnosticUrl }}</span>
              <span v-if="item.diagnosticCode !== null">状态码 {{ item.diagnosticCode }}</span>
              <span v-if="item.diagnosticLatencyMs !== null">耗时 {{ item.diagnosticLatencyMs }} ms</span>
            </div>
          </div>

          <dl class="mt-4 space-y-3 text-sm">
            <div>
              <dt class="text-xs text-muted-foreground">连接信息</dt>
              <dd class="mt-1 break-all text-foreground">{{ item.connectionSummary }}</dd>
            </div>
            <div>
              <dt class="text-xs text-muted-foreground">站点地址</dt>
              <dd class="mt-1 break-all text-foreground">{{ item.httpHost || '-' }}</dd>
            </div>
            <div class="grid grid-cols-2 gap-3">
              <div>
                <dt class="text-xs text-muted-foreground">元数据文件数</dt>
                <dd class="mt-1 text-foreground">{{ item.fileCount }}</dd>
              </div>
              <div>
                <dt class="text-xs text-muted-foreground">总记录数</dt>
                <dd class="mt-1 text-foreground">{{ item.totalRecordCount }}</dd>
              </div>
            </div>
            <div class="grid grid-cols-2 gap-3">
              <div>
                <dt class="text-xs text-muted-foreground">最近更新时间</dt>
                <dd class="mt-1 text-foreground">{{ formatDateTime(item.latestUpdatedAt) }}</dd>
              </div>
              <div>
                <dt class="text-xs text-muted-foreground">元数据来源</dt>
                <dd class="mt-1 text-foreground">{{ item.metadataSourceLabel }}</dd>
              </div>
            </div>
            <div>
              <dt class="text-xs text-muted-foreground">站点说明</dt>
              <dd class="mt-1 text-foreground">{{ item.notes || '暂无说明' }}</dd>
            </div>
          </dl>

          <div
            v-if="item.metadataMessage"
            class="mt-4 rounded-lg border border-amber-500/20 bg-amber-500/5 px-3 py-2 text-xs text-amber-700"
          >
            被动状态 · {{ item.metadataMessage }}
          </div>
        </article>
      </div>
    </div>
  </section>
</template>
