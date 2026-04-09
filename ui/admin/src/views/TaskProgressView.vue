<script setup lang="ts">
import { onMounted } from 'vue'
import { useTasksStore } from '@/stores/tasks'
import { usePolling } from '@/composables/usePolling'

const tasksStore = useTasksStore()

const { start: startPolling } = usePolling(async () => {
  await tasksStore.fetchTasks()
}, 5000)

onMounted(async () => {
  await tasksStore.fetchTasks()
  startPolling()
})

const statusColors: Record<string, string> = {
  pending: 'bg-muted text-muted-foreground',
  running: 'bg-blue-100 text-blue-800 dark:bg-blue-900 dark:text-blue-200',
  completed: 'bg-green-100 text-green-800 dark:bg-green-900 dark:text-green-200',
  failed: 'bg-red-100 text-red-800 dark:bg-red-900 dark:text-red-200',
  cancelled: 'bg-muted text-muted-foreground line-through',
}

const typeLabels: Record<string, string> = {
  parse: '数据解析',
  gen_model: '模型生成',
  export: '模型导出',
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
      class="text-center py-12 text-muted-foreground">暂无任务</div>

    <div v-else class="rounded-lg border border-border">
      <table class="w-full text-sm">
        <thead>
          <tr class="border-b border-border bg-muted/50">
            <th class="px-4 py-3 text-left font-medium text-muted-foreground">名称</th>
            <th class="px-4 py-3 text-left font-medium text-muted-foreground">类型</th>
            <th class="px-4 py-3 text-left font-medium text-muted-foreground">状态</th>
            <th class="px-4 py-3 text-left font-medium text-muted-foreground">进度</th>
            <th class="px-4 py-3 text-left font-medium text-muted-foreground">创建时间</th>
            <th class="px-4 py-3 text-right font-medium text-muted-foreground">操作</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="task in tasksStore.tasks" :key="task.id" class="border-b border-border last:border-0">
            <td class="px-4 py-3 font-medium">{{ task.name }}</td>
            <td class="px-4 py-3">{{ typeLabels[task.type] ?? task.type }}</td>
            <td class="px-4 py-3">
              <span class="inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium" :class="statusColors[task.status]">
                {{ task.status }}
              </span>
            </td>
            <td class="px-4 py-3">
              <div class="flex items-center gap-2">
                <div class="h-2 flex-1 overflow-hidden rounded-full bg-muted">
                  <div class="h-full rounded-full bg-primary transition-all" :style="{ width: `${task.progress_percent}%` }" />
                </div>
                <span class="text-xs text-muted-foreground">{{ task.progress_percent }}%</span>
              </div>
            </td>
            <td class="px-4 py-3 text-muted-foreground">{{ task.created_at }}</td>
            <td class="px-4 py-3 text-right">
              <button v-if="task.status === 'running'" @click="tasksStore.cancelTask(task.id)"
                class="text-xs text-destructive hover:underline">取消</button>
              <button v-if="task.status === 'failed'" @click="tasksStore.retryTask(task.id)"
                class="text-xs text-primary hover:underline">重试</button>
            </td>
          </tr>
        </tbody>
      </table>
    </div>
  </div>
</template>
