<script setup lang="ts">
import { computed } from 'vue'
import type { CollaborationFailedTask } from '@/types/collaboration'

const props = withDefaults(
  defineProps<{
    items: CollaborationFailedTask[]
    loading?: boolean
  }>(),
  { loading: false },
)

const emit = defineEmits<{
  retry: [id: string]
  cleanup: []
  inspect: [id: string]
}>()

const pendingCount = computed(() => props.items.filter((t) => t.retry_count < t.max_retries).length)
const exhaustedCount = computed(() => props.items.filter((t) => t.retry_count >= t.max_retries).length)

const TASK_TYPE_LABEL: Record<string, string> = {
  DatabaseQuery: '数据库查询失败',
  Compression: 'CBA 压缩失败',
  IncrementUpdate: '增量更新失败',
  MqttPublish: 'MQTT 推送失败',
}

function labelFor(type: string) {
  return TASK_TYPE_LABEL[type] ?? type
}
</script>

<template>
  <section class="collab-v2 collab-failed">
    <header>
      <div class="ttl">
        <span class="nm">失败任务队列</span>
        <span class="cnt">{{ items.length }} 条</span>
      </div>
      <div class="stat">
        待重试 <b class="warn">{{ pendingCount }}</b>
        <span class="sep">·</span>
        已耗尽 <b class="bad">{{ exhaustedCount }}</b>
        <button
          v-if="exhaustedCount > 0"
          class="link"
          @click="emit('cleanup')"
        >清理已耗尽</button>
      </div>
    </header>

    <div v-if="loading && !items.length" class="empty">
      <div class="skel" />
      <div class="skel" />
    </div>

    <div v-else-if="!items.length" class="empty-msg">无失败任务</div>

    <ul v-else>
      <li v-for="t in items" :key="t.id">
        <span class="tp">{{ labelFor(t.task_type) }}</span>
        <div class="err">
          <div>
            {{ t.site }}
            <span class="id">{{ t.id }}</span>
          </div>
          <div class="sub">{{ t.error }}</div>
          <div v-if="t.next_retry_at" class="sub">下次重试 · {{ t.next_retry_at }}</div>
        </div>
        <span :class="['rt', t.retry_count >= t.max_retries ? 'done' : '']">{{ t.retry_count }}/{{ t.max_retries }}</span>
        <div class="acts">
          <button
            v-if="t.retry_count < t.max_retries"
            class="btn"
            @click="emit('retry', t.id)"
          >重试</button>
          <button
            v-else
            class="btn"
            disabled
          >已耗尽</button>
          <button class="btn ghost" @click="emit('inspect', t.id)">详情</button>
        </div>
      </li>
    </ul>
  </section>
</template>

<style scoped>
.collab-failed { font-family: var(--collab-font-body); color: var(--collab-ink-900); border: 1px solid var(--collab-line); border-radius: 10px; overflow: hidden; background: #fff; }

header { display: flex; align-items: center; justify-content: space-between; padding: 10px 14px; border-bottom: 1px solid var(--collab-line); background: var(--collab-bg); font-size: 12.5px; color: var(--collab-ink-500); gap: 12px; flex-wrap: wrap; }
header .ttl { display: inline-flex; align-items: center; gap: 8px; }
header .nm { color: var(--collab-ink-900); font-weight: 500; }
header .cnt { font-family: var(--collab-font-mono); font-variant-numeric: tabular-nums; }
header .stat { display: inline-flex; align-items: center; gap: 6px; font-family: var(--collab-font-mono); font-size: 11.5px; font-variant-numeric: tabular-nums; }
header .stat b { font-weight: 500; }
header .stat .warn { color: var(--collab-warn); }
header .stat .bad { color: var(--collab-bad); }
header .stat .sep { color: var(--collab-ink-300); }
header .link { border: 0; background: transparent; color: var(--collab-brand); font-size: 11.5px; cursor: pointer; margin-left: 4px; padding: 0; font-family: inherit; }
header .link:hover { text-decoration: underline; }

.empty, .empty-msg { padding: 18px; color: var(--collab-ink-500); font-size: 12.5px; text-align: center; }
.empty { display: grid; grid-template-columns: repeat(auto-fit, minmax(240px, 1fr)); gap: 8px; }
.skel { height: 44px; background: var(--collab-line-soft); border-radius: 6px; animation: collab-failed-pulse 1.2s ease-in-out infinite; }
@keyframes collab-failed-pulse { 0%, 100% { opacity: .5 } 50% { opacity: 1 } }

ul { list-style: none; margin: 0; padding: 0; }
li { padding: 11px 14px; border-bottom: 1px solid var(--collab-line-soft); display: grid; grid-template-columns: 120px 1fr auto auto; gap: 12px; align-items: center; font-size: 12.5px; }
li:last-child { border-bottom: 0; }
.tp { font-family: var(--collab-font-mono); color: var(--collab-ink-500); font-size: 11px; }
.err { min-width: 0; }
.err .id { font-family: var(--collab-font-mono); color: var(--collab-ink-500); font-size: 11px; margin-left: 8px; }
.err .sub { font-family: var(--collab-font-mono); color: var(--collab-bad); font-size: 11px; margin-top: 2px; word-break: break-all; line-height: 1.4; font-variant-numeric: tabular-nums; }
.rt { font-family: var(--collab-font-mono); font-size: 11px; color: var(--collab-warn); font-variant-numeric: tabular-nums; }
.rt.done { color: var(--collab-ink-400); }

.acts { display: inline-flex; gap: 6px; }
.btn { height: 26px; padding: 0 10px; font-size: 11px; border: 1px solid var(--collab-line); background: #fff; color: var(--collab-ink-700); border-radius: 6px; cursor: pointer; font-family: inherit; }
.btn:hover:not(:disabled) { border-color: var(--collab-ink-400); }
.btn:disabled { opacity: .5; cursor: not-allowed; }
.btn.ghost { background: transparent; color: var(--collab-ink-500); }
</style>
