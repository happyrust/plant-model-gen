import { defineStore } from 'pinia'
import { ref } from 'vue'
import { extractErrorMessage } from '@/api/client'
import { tasksApi } from '@/api/tasks'
import type { TaskInfo } from '@/types/task'

export const useTasksStore = defineStore('tasks', () => {
  const tasks = ref<TaskInfo[]>([])
  const currentTask = ref<TaskInfo | null>(null)
  const loading = ref(false)
  const error = ref('')

  async function fetchTasks(params?: { status?: string; type?: string }) {
    loading.value = true
    error.value = ''
    try {
      tasks.value = await tasksApi.list(params)
    } catch (err: unknown) {
      error.value = extractErrorMessage(err)
    } finally {
      loading.value = false
    }
  }

  async function fetchTask(id: string) {
    currentTask.value = await tasksApi.get(id)
  }

  async function createTask(payload: Record<string, unknown>) {
    const task = await tasksApi.create(payload)
    tasks.value.unshift(task)
    return task
  }

  async function cancelTask(id: string) {
    await tasksApi.cancel(id)
    await fetchTasks()
  }

  async function retryTask(id: string) {
    const task = await tasksApi.retry(id)
    await fetchTasks()
    currentTask.value = task
    return task
  }

  return {
    tasks, currentTask, loading, error,
    fetchTasks, fetchTask, createTask, cancelTask, retryTask,
  }
})
