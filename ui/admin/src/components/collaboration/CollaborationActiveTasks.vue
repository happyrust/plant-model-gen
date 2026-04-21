<script setup lang="ts">
import type { CollaborationActiveTask } from '@/types/collaboration'

withDefaults(
  defineProps<{
    items: CollaborationActiveTask[]
    loading?: boolean
    title?: string
    emptyLabel?: string
  }>(),
  {
    loading: false,
    title: '进行中',
    emptyLabel: '暂无进行中任务',
  },
)

defineEmits<{
  abort: [taskId: string]
}>()

function statusChipClass(status: CollaborationActiveTask['status']) {
  switch (status) {
    case 'Running':
      return 'Syncing'
    case 'Pending':
      return 'Idle'
    case 'Completed':
      return 'Completed'
    case 'Failed':
      return 'Error'
    case 'Cancelled':
      return 'Idle'
  }
}
</script>

<template>
  <section class="collab-v2 collab-active">
    <div class="head">
      <div class="ttl">
        {{ title }}
        <span class="cnt">{{ items.length }}</span>
      </div>
      <slot name="extra" />
    </div>

    <div v-if="loading && !items.length" class="empty">
      <div class="skel" />
      <div class="skel" />
    </div>

    <div v-else-if="!items.length" class="empty-msg">{{ emptyLabel }}</div>

    <div v-else class="grid">
      <article
        v-for="t in items"
        :key="t.task_id"
        class="card"
        :class="{ running: t.status === 'Running' }"
      >
        <div class="row">
          <div class="info">
            <div class="nm">{{ t.task_name }} · {{ t.site_name }}</div>
            <div v-if="t.file_path" class="pth">{{ t.file_path }}</div>
          </div>
          <div class="meta">
            <span :class="['chip', statusChipClass(t.status)]">
              <span class="d" />
              {{ t.status }}
            </span>
            <div class="pc">{{ Math.round(t.progress) }}%</div>
          </div>
        </div>
        <div class="bar">
          <div class="f" :style="{ width: `${Math.max(2, t.progress)}%` }" />
        </div>
        <div v-if="t.status === 'Running'" class="acts">
          <button class="btn danger" @click="$emit('abort', t.task_id)">中止</button>
        </div>
      </article>
    </div>
  </section>
</template>

<style scoped>
.collab-active { font-family: var(--collab-font-body); color: var(--collab-ink-900); }

.head { display: flex; align-items: center; justify-content: space-between; margin-bottom: 10px; }
.ttl { font-size: 12.5px; color: var(--collab-ink-500); font-weight: 500; display: inline-flex; align-items: center; gap: 8px; }
.ttl .cnt { font-family: var(--collab-font-mono); font-variant-numeric: tabular-nums; color: var(--collab-ink-900); font-weight: 500; }

.empty, .empty-msg { border: 1px dashed var(--collab-line); border-radius: 10px; background: var(--collab-bg); padding: 18px; color: var(--collab-ink-500); font-size: 12.5px; text-align: center; }
.empty { display: grid; grid-template-columns: repeat(auto-fit, minmax(240px, 1fr)); gap: 10px; padding: 10px; }
.skel { height: 64px; background: var(--collab-line-soft); border-radius: 8px; animation: collab-active-pulse 1.2s ease-in-out infinite; }
@keyframes collab-active-pulse { 0%, 100% { opacity: .5 } 50% { opacity: 1 } }

.grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(260px, 1fr)); gap: 10px; }

.card { border: 1px solid var(--collab-line); border-radius: 10px; padding: 12px 14px; background: #fff; display: flex; flex-direction: column; gap: 8px; }

.row { display: flex; align-items: flex-start; justify-content: space-between; gap: 12px; }
.info { flex: 1; min-width: 0; }
.nm { font-weight: 500; font-size: 13px; color: var(--collab-ink-900); }
.pth { font-family: var(--collab-font-mono); font-size: 11px; color: var(--collab-ink-500); margin-top: 2px; word-break: break-all; line-height: 1.4; font-variant-numeric: tabular-nums; }

.meta { display: flex; flex-direction: column; align-items: flex-end; gap: 4px; flex-shrink: 0; }
.pc { font-family: var(--collab-font-mono); font-size: 11px; color: var(--collab-ink-500); font-variant-numeric: tabular-nums; }

.chip { display: inline-flex; align-items: center; gap: 4px; padding: 2px 8px; border-radius: 4px; font-size: 10.5px; font-weight: 500; }
.chip .d { width: 5px; height: 5px; border-radius: 999px; background: currentColor; }
.chip.Idle, .chip.Completed { background: var(--collab-line-soft); color: var(--collab-ink-500); }
.chip.Syncing { background: var(--collab-ok-bg); color: var(--collab-ok); }
.chip.Error { background: var(--collab-bad-bg); color: var(--collab-bad); }
.chip.Syncing .d { animation: collab-active-chip-pulse 1.4s ease-in-out infinite; }
@keyframes collab-active-chip-pulse { 0%, 100% { opacity: 1 } 50% { opacity: .4 } }

.bar { height: 3px; background: var(--collab-line-soft); border-radius: 2px; overflow: hidden; }
.bar .f { height: 100%; background: var(--collab-brand); border-radius: 2px; transition: width .4s ease; }
.card.running .bar .f { background: var(--collab-ok); animation: collab-active-shimmer 1.8s linear infinite; }
@keyframes collab-active-shimmer { 0% { opacity: 1 } 50% { opacity: .55 } 100% { opacity: 1 } }

.acts { display: flex; justify-content: flex-end; gap: 6px; }
.btn { height: 26px; padding: 0 10px; font-size: 11px; border: 1px solid var(--collab-line); background: #fff; color: var(--collab-ink-700); border-radius: 6px; cursor: pointer; font-family: inherit; }
.btn:hover { border-color: var(--collab-ink-400); }
.btn.danger { border-color: var(--collab-bad); color: var(--collab-bad); }
.btn.danger:hover { background: var(--collab-bad-bg); }
</style>
