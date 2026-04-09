import { defineStore } from 'pinia'
import { ref } from 'vue'
import { tasksApi } from '@/api/tasks'
import type { Task, SubTask } from '@/types/task'

export const useTasksStore = defineStore('tasks', () => {
  const tasks = ref<Task[]>([])
  const currentTask = ref<Task | null>(null)
  const subtasks = ref<SubTask[]>([])
  const loading = ref(false)
  const error = ref('')

  async function fetchTasks(params?: { status?: string; type?: string }) {
    loading.value = true
    error.value = ''
    try {
      tasks.value = await tasksApi.list(params)
    } catch (err: unknown) {
      error.value = err instanceof Error ? err.message : 'Failed to fetch tasks'
    } finally {
      loading.value = false
    }
  }

  async function fetchTask(id: string) {
    currentTask.value = await tasksApi.get(id)
  }

  async function fetchSubtasks(taskId: string) {
    subtasks.value = await tasksApi.subtasks(taskId)
  }

  async function createTask(payload: Partial<Task>) {
    const task = await tasksApi.create(payload)
    tasks.value.unshift(task)
    return task
  }

  async function cancelTask(id: string) {
    await tasksApi.cancel(id)
    await fetchTasks()
  }

  async function retryTask(id: string) {
    await tasksApi.retry(id)
    await fetchTasks()
  }

  return {
    tasks, currentTask, subtasks, loading, error,
    fetchTasks, fetchTask, fetchSubtasks, createTask, cancelTask, retryTask,
  }
})
