<script setup lang="ts">
import { Download, PenLine, Plus } from 'lucide-vue-next'
import { formatRelativeTime } from '@/lib/collaboration'
import type { CollaborationGroupListItem, CollaborationTone } from '@/types/collaboration'

defineProps<{
  items: CollaborationGroupListItem[]
  selectedId: string | null
  loading: boolean
  error: string
  actionDisabled?: boolean
}>()

const emit = defineEmits<{
  select: [id: string]
  retry: []
  create: []
  import: []
  edit: [id: string]
}>()

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
    <div class="border-b border-border px-5 py-4">
      <div class="flex items-start justify-between gap-3">
        <div>
          <p class="text-xs font-medium uppercase tracking-[0.18em] text-muted-foreground">协同组列表</p>
          <h2 class="mt-1 text-lg font-semibold">异地协同组</h2>
          <p class="mt-1 text-sm text-muted-foreground">
            左侧用于切换协同组，右侧查看当前协同组的站点、洞察与实时日志。
          </p>
        </div>
        <div class="flex flex-col items-stretch gap-2 sm:flex-row sm:items-center">
          <button
            class="inline-flex h-9 items-center gap-2 rounded-md border border-input bg-background px-3 text-sm font-medium text-foreground transition-colors hover:bg-accent disabled:pointer-events-none disabled:opacity-50"
            :disabled="actionDisabled"
            @click="emit('import')"
          >
            <Download class="h-4 w-4" />
            导入当前配置
          </button>
          <button
            class="inline-flex h-9 items-center gap-2 rounded-md bg-primary px-3 text-sm font-medium text-primary-foreground shadow transition-colors hover:bg-primary/90 disabled:pointer-events-none disabled:opacity-50"
            :disabled="actionDisabled"
            @click="emit('create')"
          >
            <Plus class="h-4 w-4" />
            新建协同组
          </button>
        </div>
      </div>
    </div>

    <div class="p-4">
      <div v-if="loading && !items.length" class="space-y-3">
        <div
          v-for="index in 4"
          :key="index"
          class="h-28 animate-pulse rounded-xl border border-border bg-muted/50"
        />
      </div>

      <div
        v-else-if="error && !items.length"
        class="rounded-xl border border-rose-500/20 bg-rose-500/5 p-4 text-sm text-rose-600"
      >
        <div class="font-medium">协同组列表加载失败</div>
        <div class="mt-1">{{ error }}</div>
        <button
          class="mt-3 inline-flex h-9 items-center rounded-md border border-border px-3 text-sm font-medium text-foreground transition-colors hover:bg-accent"
          @click="emit('retry')"
        >
          重新加载
        </button>
      </div>

      <div
        v-else-if="!items.length"
        class="rounded-xl border border-dashed border-border bg-muted/20 px-4 py-10 text-center text-sm text-muted-foreground"
      >
        当前没有可用的协同组。
      </div>

      <div v-else class="space-y-3">
        <div
          v-if="error"
          class="rounded-lg border border-amber-500/20 bg-amber-500/5 px-3 py-2 text-xs text-amber-700"
        >
          {{ error }}
        </div>

        <article
          v-for="item in items"
          :key="item.id"
          class="rounded-xl border px-4 py-3 transition-colors"
          :class="item.id === selectedId
            ? 'border-primary bg-primary/5 shadow-sm'
            : 'border-border bg-background hover:bg-accent/20'"
        >
          <div class="flex items-start justify-between gap-3">
            <div class="min-w-0 flex-1">
              <div class="flex items-center gap-2">
                <h3 class="truncate text-sm font-semibold text-foreground">{{ item.name }}</h3>
                <span
                  class="inline-flex shrink-0 items-center rounded-full border px-2 py-0.5 text-[11px] font-medium"
                  :class="toneClass(item.statusTone)"
                >
                  {{ item.statusLabel }}
                </span>
              </div>
              <p class="mt-1 text-xs text-muted-foreground">
                {{ item.location || '未配置区域说明' }}
              </p>
            </div>
            <button
              class="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-md border border-border bg-background text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:pointer-events-none disabled:opacity-50"
              :disabled="actionDisabled"
              @click="emit('edit', item.id)"
            >
              <PenLine class="h-4 w-4" />
            </button>
          </div>

          <button
            class="mt-3 w-full rounded-lg bg-muted/40 px-3 py-2 text-left text-xs text-muted-foreground transition-colors hover:bg-muted/70"
            @click="emit('select', item.id)"
          >
            <div class="truncate">{{ item.mqttSummary }}</div>
            <div class="mt-1 flex items-center justify-between gap-3">
              <span>站点 {{ item.siteCount }}</span>
              <span>{{ formatRelativeTime(item.updatedAt) }}</span>
            </div>
          </button>
        </article>
      </div>
    </div>
  </section>
</template>
