<script setup lang="ts">
import { ref, watch } from 'vue'
import { X } from 'lucide-vue-next'
import type { CollaborationConfig } from '@/types/collaboration'

const props = defineProps<{
  open: boolean
  config: CollaborationConfig
  envLabel?: string | null
  disabled?: boolean
  save: (next: CollaborationConfig) => Promise<void> | void
}>()

const emit = defineEmits<{
  close: []
}>()

const saving = ref(false)
const error = ref('')
const draft = ref<CollaborationConfig>({ ...props.config })

watch(
  () => props.open,
  (open) => {
    if (!open) return
    error.value = ''
    draft.value = { ...props.config }
  },
)

function onScrimClick() {
  if (!saving.value) emit('close')
}

function patch<K extends keyof CollaborationConfig>(key: K, value: CollaborationConfig[K]) {
  draft.value = { ...draft.value, [key]: value }
}

function toggle(key: 'auto_detect' | 'auto_sync' | 'enable_notifications') {
  patch(key, !draft.value[key])
}

async function onSubmit() {
  if (saving.value || props.disabled) return
  saving.value = true
  error.value = ''
  try {
    await props.save({ ...draft.value })
    emit('close')
  } catch (err: unknown) {
    error.value = err instanceof Error ? err.message : '保存配置失败'
  } finally {
    saving.value = false
  }
}

function onKey(e: KeyboardEvent) {
  if (e.key === 'Escape' && !saving.value) emit('close')
}
</script>

<template>
  <Teleport to="body">
    <Transition name="collab-cfg">
      <div v-if="open" class="collab-v2 cfg-overlay" @keydown="onKey" tabindex="-1">
        <div class="scrim" @click="onScrimClick" />
        <aside class="drawer" role="dialog" aria-labelledby="collab-cfg-title">
          <header class="hdr">
            <div class="ti">
              <h3 id="collab-cfg-title"><em>参数配置</em></h3>
              <div class="loc">作用范围 · <code>{{ envLabel || '当前协同组' }}</code></div>
            </div>
            <button class="close" :disabled="saving" aria-label="关闭" @click="emit('close')">
              <X class="h-3.5 w-3.5" />
            </button>
          </header>

          <div class="body">
            <section>
              <div class="sh">自动化</div>
              <div class="row">
                <div class="l">
                  <div class="nm">自动检测变更</div>
                  <div class="hp">按下面的间隔轮询各 Peer，发现增量则入队。</div>
                </div>
                <button :class="['switch', draft.auto_detect ? 'on' : '']" @click="toggle('auto_detect')" aria-label="自动检测" />
              </div>
              <div class="row">
                <div class="l">
                  <div class="nm">检测间隔 (秒)</div>
                  <div class="hp">推荐 30–300 秒，过短会打爆对端文件服务。</div>
                </div>
                <input
                  type="number"
                  min="5"
                  :value="draft.detect_interval"
                  @input="patch('detect_interval', Number(($event.target as HTMLInputElement).value || 0))"
                />
              </div>
              <div class="row">
                <div class="l">
                  <div class="nm">检测后自动同步</div>
                  <div class="hp">发现变更立即拉取；生产环境建议关闭由人工触发。</div>
                </div>
                <button :class="['switch', draft.auto_sync ? 'on' : '']" @click="toggle('auto_sync')" aria-label="自动同步" />
              </div>
            </section>

            <section>
              <div class="sh">吞吐与并发</div>
              <div class="row">
                <div class="l">
                  <div class="nm">批次大小 (文件/批)</div>
                  <div class="hp">单批同步的最大文件数；大批次降低开销但内存占用更高。</div>
                </div>
                <input
                  type="number"
                  min="1"
                  :value="draft.batch_size"
                  @input="patch('batch_size', Number(($event.target as HTMLInputElement).value || 0))"
                />
              </div>
              <div class="row">
                <div class="l">
                  <div class="nm">最大并发同步</div>
                  <div class="hp">跨站点同时进行的 sync 任务数。</div>
                </div>
                <input
                  type="number"
                  min="1"
                  max="16"
                  :value="draft.max_concurrent"
                  @input="patch('max_concurrent', Number(($event.target as HTMLInputElement).value || 0))"
                />
              </div>
            </section>

            <section>
              <div class="sh">连接与重连</div>
              <div class="row">
                <div class="l">
                  <div class="nm">重连初始间隔 (ms)</div>
                  <div class="hp">首次断线后等待多久开始重连。</div>
                </div>
                <input
                  type="number"
                  min="100"
                  :value="draft.reconnect_initial_ms"
                  @input="patch('reconnect_initial_ms', Number(($event.target as HTMLInputElement).value || 0))"
                />
              </div>
              <div class="row">
                <div class="l">
                  <div class="nm">重连最大间隔 (ms)</div>
                  <div class="hp">指数退避的上限，防止无限退避。</div>
                </div>
                <input
                  type="number"
                  min="1000"
                  :value="draft.reconnect_max_ms"
                  @input="patch('reconnect_max_ms', Number(($event.target as HTMLInputElement).value || 0))"
                />
              </div>
            </section>

            <section>
              <div class="sh">通知与日志</div>
              <div class="row">
                <div class="l">
                  <div class="nm">桌面通知</div>
                  <div class="hp">浏览器 Notification API，失败/完成时推送。</div>
                </div>
                <button :class="['switch', draft.enable_notifications ? 'on' : '']" @click="toggle('enable_notifications')" aria-label="通知" />
              </div>
              <div class="row">
                <div class="l">
                  <div class="nm">日志保留天数</div>
                  <div class="hp">超过此天数的日志由后台清理。</div>
                </div>
                <input
                  type="number"
                  min="1"
                  :value="draft.log_retention_days"
                  @input="patch('log_retention_days', Number(($event.target as HTMLInputElement).value || 0))"
                />
              </div>
            </section>

            <div v-if="error" class="err">{{ error }}</div>
          </div>

          <footer class="foot">
            <div class="sp" />
            <button class="btn" :disabled="saving" @click="emit('close')">取消</button>
            <button class="btn primary" :disabled="saving || disabled" @click="onSubmit">
              {{ saving ? '保存中…' : '保存配置' }}
            </button>
          </footer>
        </aside>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.cfg-overlay { position: fixed; inset: 0; z-index: 50; font-family: var(--collab-font-body); color: var(--collab-ink-900); }
.scrim { position: absolute; inset: 0; background: rgba(15, 23, 42, .32); }
.drawer {
  position: absolute; top: 0; right: 0; height: 100vh; width: 560px; max-width: 94vw;
  background: var(--collab-bg); border-left: 1px solid var(--collab-line);
  box-shadow: -24px 0 48px -16px rgba(15, 23, 42, .14);
  display: flex; flex-direction: column;
}

.collab-cfg-enter-active, .collab-cfg-leave-active { transition: opacity .22s ease; }
.collab-cfg-enter-from, .collab-cfg-leave-to { opacity: 0; }
.collab-cfg-enter-active .drawer, .collab-cfg-leave-active .drawer { transition: transform .24s cubic-bezier(.32, .72, 0, 1); }
.collab-cfg-enter-from .drawer, .collab-cfg-leave-to .drawer { transform: translateX(100%); }

.hdr { display: flex; align-items: flex-start; gap: 14px; padding: 20px 24px 16px; border-bottom: 1px solid var(--collab-line); }
.ti { flex: 1; min-width: 0; }
h3 { font-family: var(--collab-font-display); font-weight: 500; font-size: 24px; margin: 0; letter-spacing: -.01em; line-height: 1.2; }
h3 em { font-style: italic; color: var(--collab-brand); }
.loc { margin-top: 4px; color: var(--collab-ink-500); font-size: 12.5px; }
.loc code { font-family: var(--collab-font-mono); font-size: 11.5px; color: var(--collab-ink-700); background: var(--collab-line-soft); padding: 1px 6px; border-radius: 4px; }
.close { height: 32px; width: 32px; border: 1px solid var(--collab-line); border-radius: 8px; background: #fff; display: inline-flex; align-items: center; justify-content: center; cursor: pointer; color: var(--collab-ink-500); }
.close:hover:not(:disabled) { border-color: var(--collab-ink-400); color: var(--collab-ink-900); }

.body { flex: 1; overflow: auto; padding: 6px 24px 80px; }
section { margin-top: 22px; }
.sh { display: flex; align-items: center; gap: 10px; font-size: 11.5px; color: var(--collab-ink-500); text-transform: uppercase; letter-spacing: .08em; margin-bottom: 10px; }
.sh::after { content: ""; flex: 1; height: 1px; background: var(--collab-line); }
.row { display: grid; grid-template-columns: 1fr 120px; gap: 16px; align-items: center; padding: 12px 14px; border: 1px solid var(--collab-line); border-radius: 10px; background: #fff; margin-bottom: 8px; }
.l { display: flex; flex-direction: column; gap: 4px; }
.l .nm { font-size: 13px; font-weight: 500; color: var(--collab-ink-900); }
.l .hp { font-size: 11.5px; color: var(--collab-ink-500); }
.row input[type="number"] { height: 32px; border: 1px solid var(--collab-line); border-radius: 8px; padding: 0 10px; background: #fff; font-family: var(--collab-font-mono); font-size: 12px; color: var(--collab-ink-900); outline: none; font-variant-numeric: tabular-nums; width: 100%; }
.row input:focus { border-color: var(--collab-brand); }

.switch { position: relative; display: inline-flex; align-items: center; width: 42px; height: 22px; border-radius: 999px; background: var(--collab-line); cursor: pointer; transition: background .2s; margin-left: auto; justify-self: end; border: 0; padding: 0; }
.switch.on { background: var(--collab-brand); }
.switch::after { content: ""; position: absolute; top: 2px; left: 2px; width: 18px; height: 18px; background: #fff; border-radius: 999px; transition: transform .2s; box-shadow: 0 1px 3px rgba(0, 0, 0, .2); }
.switch.on::after { transform: translateX(20px); }

.err { margin-top: 14px; padding: 10px 12px; border-radius: 8px; background: var(--collab-bad-bg); color: var(--collab-bad); font-size: 12px; border: 1px solid color-mix(in oklch, var(--collab-bad) 30%, transparent); }

.foot { border-top: 1px solid var(--collab-line); background: var(--collab-bg); padding: 14px 24px; display: flex; gap: 10px; }
.foot .sp { flex: 1; }
.btn { height: 36px; padding: 0 16px; font-size: 13px; border: 1px solid var(--collab-line); background: #fff; color: var(--collab-ink-700); border-radius: 8px; cursor: pointer; font-family: inherit; font-weight: 500; transition: border-color .15s, background .15s; }
.btn:hover:not(:disabled) { border-color: var(--collab-ink-400); }
.btn:disabled { opacity: .5; cursor: not-allowed; }
.btn.primary { background: var(--collab-brand); color: #fff; border-color: var(--collab-brand); }
.btn.primary:hover:not(:disabled) { background: var(--collab-brand-strong); border-color: var(--collab-brand-strong); }
</style>
