<script setup lang="ts">
/**
 * 站点删除二次确认对话框（D2 / Sprint D · 修 G9）
 *
 * 替代之前的 `window.confirm`，给"删除站点"动作一个 hi-fi、可关闭、可
 * 复制项目名的对话框，防止误删。
 *
 * 关键 UX：
 * - 显示项目名 + site_id，让操作者一眼看到正在删除的对象
 * - 复制项目名到剪贴板的快捷按钮（运维场景常需把 site_id 贴到工单）
 * - 主动作（"确认删除"）走危险色 + 主键盘焦点，避免 Enter 误触
 * - 按 Esc / 点遮罩 / 点关闭都能取消
 *
 * 与设计原型 `design/site-admin-flow-demo/PLAN.md §5 Phase E` 的
 * "删除二次确认弹框" 对齐。
 */
import { computed } from 'vue'
import { AlertTriangle, Copy, X } from 'lucide-vue-next'
import type { ManagedProjectSite } from '@/types/site'

const props = defineProps<{
  open: boolean
  site: ManagedProjectSite | null
  pending: boolean
}>()

const emit = defineEmits<{
  cancel: []
  confirm: []
}>()

const projectName = computed(() => props.site?.project_name ?? '')
const siteId = computed(() => props.site?.site_id ?? '')

function copySiteId() {
  if (!siteId.value) return
  navigator.clipboard.writeText(siteId.value).catch(() => {
    /* clipboard 在某些 file:// 上下文下可能拒绝，忽略即可 */
  })
}

function onMaskClick() {
  if (props.pending) return
  emit('cancel')
}
</script>

<template>
  <Teleport to="body">
    <Transition name="dialog">
      <div v-if="open && site" class="fixed inset-0 z-50">
        <div class="absolute inset-0 bg-black/50" @click="onMaskClick" />
        <div
          role="dialog"
          aria-modal="true"
          aria-labelledby="site-delete-title"
          class="absolute left-1/2 top-1/2 w-full max-w-[440px] -translate-x-1/2 -translate-y-1/2 rounded-lg border border-border bg-background shadow-xl"
        >
          <div class="flex items-start justify-between gap-3 border-b border-border px-6 py-4">
            <div class="flex items-start gap-3">
              <span class="inline-flex h-10 w-10 shrink-0 items-center justify-center rounded-full bg-destructive/10 text-destructive">
                <AlertTriangle class="h-5 w-5" />
              </span>
              <div>
                <h3 id="site-delete-title" class="text-lg font-semibold">删除站点</h3>
                <p class="text-sm text-muted-foreground">此操作不可撤销，受管子进程及其数据目录会被一并清理</p>
              </div>
            </div>
            <button
              :disabled="pending"
              class="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-md text-muted-foreground hover:bg-accent transition-colors disabled:pointer-events-none disabled:opacity-50"
              @click="emit('cancel')"
              aria-label="关闭"
            >
              <X class="h-4 w-4" />
            </button>
          </div>

          <div class="space-y-3 px-6 py-4">
            <div class="rounded-md border border-border bg-muted/40 px-3 py-2">
              <div class="text-xs uppercase tracking-wider text-muted-foreground">项目名称</div>
              <div class="mt-0.5 text-sm font-medium">{{ projectName || '(未命名)' }}</div>
            </div>
            <div class="rounded-md border border-border bg-muted/40 px-3 py-2">
              <div class="flex items-center justify-between">
                <div>
                  <div class="text-xs uppercase tracking-wider text-muted-foreground">Site ID</div>
                  <div class="mt-0.5 font-mono text-sm">{{ siteId }}</div>
                </div>
                <button
                  type="button"
                  class="inline-flex h-7 items-center gap-1.5 rounded-md border border-border px-2 text-xs hover:bg-accent transition-colors"
                  :disabled="pending || !siteId"
                  @click="copySiteId"
                >
                  <Copy class="h-3.5 w-3.5" /> 复制
                </button>
              </div>
            </div>

            <p class="text-sm text-muted-foreground">
              确认要删除该站点吗？相关 SurrealDB / web_server 子进程会被停止，
              <code class="rounded bg-muted px-1 py-0.5 text-xs">runtime/admin_sites/&lt;site_id&gt;</code>
              目录中的数据也会被清理。
            </p>
          </div>

          <div class="flex justify-end gap-3 border-t border-border bg-muted/30 px-6 py-4">
            <button
              type="button"
              :disabled="pending"
              class="inline-flex h-9 items-center rounded-md border border-input bg-transparent px-4 text-sm font-medium hover:bg-accent transition-colors disabled:pointer-events-none disabled:opacity-50"
              @click="emit('cancel')"
            >
              取消
            </button>
            <button
              type="button"
              :disabled="pending"
              class="inline-flex h-9 items-center gap-2 rounded-md bg-destructive px-4 text-sm font-medium text-destructive-foreground shadow hover:bg-destructive/90 transition-colors disabled:pointer-events-none disabled:opacity-50"
              @click="emit('confirm')"
            >
              <span v-if="pending" class="h-3.5 w-3.5 animate-spin rounded-full border-2 border-destructive-foreground/40 border-t-transparent" />
              {{ pending ? '删除中...' : '确认删除' }}
            </button>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.dialog-enter-active,
.dialog-leave-active {
  transition: all 0.2s ease;
}
.dialog-enter-active > div:first-child,
.dialog-leave-active > div:first-child {
  transition: opacity 0.2s;
}
.dialog-enter-active > div:last-child,
.dialog-leave-active > div:last-child {
  transition: transform 0.2s ease, opacity 0.2s;
}
.dialog-enter-from > div:first-child,
.dialog-leave-to > div:first-child {
  opacity: 0;
}
.dialog-enter-from > div:last-child,
.dialog-leave-to > div:last-child {
  opacity: 0;
  transform: translate(-50%, -50%) scale(0.96);
}
</style>
