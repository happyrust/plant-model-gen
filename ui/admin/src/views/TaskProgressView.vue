<script setup lang="ts">
import { ref, onMounted, computed } from 'vue'
import { useRoute } from 'vue-router'
import { useTasksStore } from '@/stores/tasks'
import { usePolling } from '@/composables/usePolling'
import { getTaskTypeLabel, PRIORITY_LABELS } from '@/types/task'
import type { TaskInfo, TaskStatus } from '@/types/task'
import { X, ChevronRight } from 'lucide-vue-next'

const route = useRoute()
const tasksStore = useTasksStore()
const detailTask = ref<TaskInfo | null>(null)
const showDetail = ref(false)

const hasRunning = computed(() => tasksStore.tasks.some((t) => t.status === 'Running'))

const { start: startPolling } = usePolling(async () => {
  await tasksStore.fetchTasks()
}, hasRunning.value ? 3000 : 10000)

onMounted(async () => {
  await tasksStore.fetchTasks()
  startPolling()
  const taskId = route.params.id as string | undefined
  if (taskId) openDetail(taskId)
})

async function handleRetry(id: string) {
  const retried = await tasksStore.retryTask(id)
  await openDetail(retried.id)
}

async function openDetail(id: string) {
  await tasksStore.fetchTask(id)
  detailTask.value = tasksStore.currentTask
  showDetail.value = true
}

function closeDetail() {
  showDetail.value = false
  detailTask.value = null
}

function formatTimestamp(ts: number | null | undefined): string {
  if (!ts) return '-'
  return new Date(ts).toLocaleString('zh-CN')
}

function formatDuration(ms: number | null | undefined): string {
  if (!ms) return '-'
  const sec = Math.floor(ms / 1000)
  if (sec < 60) return `${sec}s`
  const min = Math.floor(sec / 60)
  return `${min}m ${sec % 60}s`
}

const statusConfig: Record<TaskStatus, { class: string; label: string }> = {
  Pending: { class: 'bg-muted text-muted-foreground', label: '等待中' },
  Running: { class: 'bg-blue-100 text-blue-800 dark:bg-blue-900 dark:text-blue-200', label: '运行中' },
  Completed: { class: 'bg-green-100 text-green-800 dark:bg-green-900 dark:text-green-200', label: '已完成' },
  Failed: { class: 'bg-red-100 text-red-800 dark:bg-red-900 dark:text-red-200', label: '失败' },
  Cancelled: { class: 'bg-muted text-muted-foreground line-through', label: '已取消' },
}
</script>

<template>
  <div class="space-y-6">
    <div class="flex items-center justify-between">
      <div>
        <h2 class="text-2xl font-semibold tracking-tight">任务进度</h2>
        <p class="text-sm text-muted-foreground">查看和管理任务执行状态</p>
      </div>
      <router-link to="/tasks/new"
        class="inline-flex h-9 items-center gap-2 rounded-md bg-primary px-4 text-sm font-medium text-primary-foreground shadow hover:bg-primary/90 transition-colors">
        + 新建任务
      </router-link>
    </div>

    <div v-if="tasksStore.loading && !tasksStore.tasks.length"
      class="text-center py-12 text-muted-foreground">加载中...</div>
    <div v-else-if="!tasksStore.tasks.length"
      class="text-center py-12 text-muted-foreground">暂无任务，点击右上角创建</div>

    <div v-else class="rounded-lg border border-border">
      <table class="w-full text-sm">
        <thead>
          <tr class="border-b border-border bg-muted/50">
            <th class="px-4 py-3 text-left font-medium text-muted-foreground">名称</th>
            <th class="px-4 py-3 text-left font-medium text-muted-foreground">类型</th>
            <th class="px-4 py-3 text-left font-medium text-muted-foreground">状态</th>
            <th class="px-4 py-3 text-left font-medium text-muted-foreground">进度</th>
            <th class="px-4 py-3 text-left font-medium text-muted-foreground">创建时间</th>
            <th class="px-4 py-3 text-left font-medium text-muted-foreground">耗时</th>
            <th class="px-4 py-3 text-right font-medium text-muted-foreground">操作</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="task in tasksStore.tasks" :key="task.id"
            class="border-b border-border last:border-0 hover:bg-muted/30 transition-colors cursor-pointer"
            @click="openDetail(task.id)">
            <td class="px-4 py-3">
              <div class="font-medium">{{ task.name }}</div>
              <div class="text-xs text-muted-foreground">{{ task.id }}</div>
            </td>
            <td class="px-4 py-3">{{ getTaskTypeLabel(task.task_type) }}</td>
            <td class="px-4 py-3">
              <span class="inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium"
                :class="statusConfig[task.status]?.class">
                {{ statusConfig[task.status]?.label ?? task.status }}
              </span>
            </td>
            <td class="px-4 py-3">
              <div class="flex items-center gap-2">
                <div class="h-2 w-24 overflow-hidden rounded-full bg-muted">
                  <div class="h-full rounded-full bg-primary transition-all"
                    :style="{ width: `${task.progress.percentage}%` }" />
                </div>
                <span class="text-xs text-muted-foreground w-10 text-right">{{ Math.round(task.progress.percentage) }}%</span>
              </div>
              <div class="text-xs text-muted-foreground mt-0.5">{{ task.progress.current_step }}</div>
            </td>
            <td class="px-4 py-3 text-muted-foreground">{{ formatTimestamp(task.created_at) }}</td>
            <td class="px-4 py-3 text-muted-foreground">{{ formatDuration(task.actual_duration) }}</td>
            <td class="px-4 py-3 text-right" @click.stop>
              <div class="flex items-center justify-end gap-1">
                <button v-if="task.status === 'Failed'" @click="handleRetry(task.id)"
                  class="text-xs text-primary hover:underline px-2 py-1">重试</button>
                <ChevronRight class="h-4 w-4 text-muted-foreground" />
              </div>
            </td>
          </tr>
        </tbody>
      </table>
    </div>

    <!-- Detail Dialog -->
    <Teleport to="body">
      <Transition name="drawer">
        <div v-if="showDetail && detailTask" class="fixed inset-0 z-50">
          <div class="absolute inset-0 bg-black/50" @click="closeDetail" />
          <div class="absolute right-0 top-0 h-full w-full max-w-[560px] bg-background border-l border-border shadow-xl flex flex-col overflow-hidden">
            <div class="flex items-center justify-between border-b border-border px-6 py-4">
              <div>
                <h3 class="text-lg font-semibold">{{ detailTask.name }}</h3>
                <div class="flex items-center gap-2 mt-1">
                  <span class="inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium"
                    :class="statusConfig[detailTask.status]?.class">
                    {{ statusConfig[detailTask.status]?.label }}
                  </span>
                  <span class="text-xs text-muted-foreground">{{ PRIORITY_LABELS[detailTask.priority] }}</span>
                </div>
              </div>
              <button @click="closeDetail"
                class="inline-flex h-8 w-8 items-center justify-center rounded-md hover:bg-accent transition-colors">
                <X class="h-4 w-4" />
              </button>
            </div>

            <div class="flex-1 overflow-auto px-6 py-4 space-y-4">
              <!-- Progress -->
              <div class="rounded-lg border border-border p-4">
                <div class="flex items-center justify-between mb-2">
                  <span class="text-sm font-medium">进度</span>
                  <span class="text-sm text-muted-foreground">{{ Math.round(detailTask.progress.percentage) }}%</span>
                </div>
                <div class="h-3 overflow-hidden rounded-full bg-muted">
                  <div class="h-full rounded-full bg-primary transition-all" :style="{ width: `${detailTask.progress.percentage}%` }" />
                </div>
                <div class="mt-2 grid grid-cols-2 gap-2 text-xs text-muted-foreground">
                  <div>当前步骤: {{ detailTask.progress.current_step }}</div>
                  <div>步骤 {{ detailTask.progress.current_step_number }} / {{ detailTask.progress.total_steps }}</div>
                  <div>已处理: {{ detailTask.progress.processed_items }} / {{ detailTask.progress.total_items }}</div>
                  <div v-if="detailTask.progress.estimated_remaining_seconds">
                    预计剩余: {{ Math.ceil(detailTask.progress.estimated_remaining_seconds / 60) }}m
                  </div>
                </div>
              </div>

              <!-- Info -->
              <div class="rounded-lg border border-border p-4">
                <h4 class="text-sm font-medium mb-3">任务信息</h4>
                <div class="grid grid-cols-2 gap-y-2 text-sm">
                  <div class="text-muted-foreground">ID</div><div class="font-mono text-xs">{{ detailTask.id }}</div>
                  <div class="text-muted-foreground">类型</div><div>{{ getTaskTypeLabel(detailTask.task_type) }}</div>
                  <div class="text-muted-foreground">创建时间</div><div>{{ formatTimestamp(detailTask.created_at) }}</div>
                  <div class="text-muted-foreground">开始时间</div><div>{{ formatTimestamp(detailTask.started_at) }}</div>
                  <div class="text-muted-foreground">完成时间</div><div>{{ formatTimestamp(detailTask.completed_at) }}</div>
                  <div class="text-muted-foreground">耗时</div><div>{{ formatDuration(detailTask.actual_duration) }}</div>
                </div>
              </div>

              <!-- Error -->
              <div v-if="detailTask.error" class="rounded-lg border border-destructive/50 bg-destructive/5 p-4">
                <h4 class="text-sm font-medium text-destructive mb-2">错误信息</h4>
                <div class="text-sm text-destructive/80">{{ detailTask.error }}</div>
                <div v-if="detailTask.error_details?.suggested_solutions?.length" class="mt-3">
                  <div class="text-xs text-muted-foreground mb-1">建议解决方案:</div>
                  <ul class="list-disc list-inside text-xs text-muted-foreground space-y-0.5">
                    <li v-for="(s, i) in detailTask.error_details.suggested_solutions" :key="i">{{ s }}</li>
                  </ul>
                </div>
              </div>

              <!-- Logs -->
              <div v-if="detailTask.logs.length" class="rounded-lg border border-border">
                <div class="border-b border-border px-4 py-2">
                  <h4 class="text-sm font-medium">日志 ({{ detailTask.logs.length }})</h4>
                </div>
                <div class="max-h-60 overflow-auto p-3">
                  <div v-for="(log, i) in detailTask.logs" :key="i" class="flex gap-2 text-xs py-0.5">
                    <span class="shrink-0 text-muted-foreground w-16">{{ formatTimestamp(log.timestamp).split(' ')[1] ?? '' }}</span>
                    <span class="shrink-0 font-medium"
                      :class="log.level === 'Error' || log.level === 'Critical' ? 'text-destructive' :
                               log.level === 'Warning' ? 'text-amber-600' : 'text-muted-foreground'">
                      [{{ log.level }}]
                    </span>
                    <span class="break-all">{{ log.message }}</span>
                  </div>
                </div>
              </div>
            </div>
          </div>
        </div>
      </Transition>
    </Teleport>
  </div>
</template>

<style scoped>
.drawer-enter-active,
.drawer-leave-active {
  transition: all 0.3s ease;
}
.drawer-enter-active > div:first-child,
.drawer-leave-active > div:first-child {
  transition: opacity 0.3s;
}
.drawer-enter-active > div:last-child,
.drawer-leave-active > div:last-child {
  transition: transform 0.3s ease;
}
.drawer-enter-from > div:first-child,
.drawer-leave-to > div:first-child {
  opacity: 0;
}
.drawer-enter-from > div:last-child,
.drawer-leave-to > div:last-child {
  transform: translateX(100%);
}
</style>
