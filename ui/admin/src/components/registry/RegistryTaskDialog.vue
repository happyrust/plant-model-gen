<script setup lang="ts">
import { ref, watch } from 'vue'
import { X } from 'lucide-vue-next'

import { useRegistryStore } from '@/stores/registry'
import { PRIORITY_LABELS, TASK_TYPE_LABELS, type DatabaseConfig, type TaskPriority, type TaskType } from '@/types/task'
import type { RegistrySite } from '@/types/registry'

const props = defineProps<{
  open: boolean
  site: RegistrySite | null
}>()

const emit = defineEmits<{
  close: []
  created: [payload: { taskId: string; message: string; siteName: string }]
}>()

const registryStore = useRegistryStore()

const saving = ref(false)
const error = ref('')
const taskName = ref('')
const taskType = ref<TaskType>('ParsePdmsData')
const priority = ref<TaskPriority>('Normal')
const configOverrideText = ref('')

const taskTypeOptions: TaskType[] = [
  'ParsePdmsData',
  'DataGeneration',
  'FullGeneration',
]

watch(
  () => [props.open, props.site?.site_id] as const,
  ([open]) => {
    if (!open || !props.site) return
    error.value = ''
    taskType.value = 'ParsePdmsData'
    priority.value = 'Normal'
    taskName.value = `${props.site.name || props.site.project_name} - ${TASK_TYPE_LABELS.ParsePdmsData}`
    configOverrideText.value = JSON.stringify(props.site.config ?? {}, null, 2)
  },
  { immediate: true },
)

function parseConfigOverride(): DatabaseConfig | null {
  const text = configOverrideText.value.trim()
  if (!text) return null

  let parsed: unknown
  try {
    parsed = JSON.parse(text)
  } catch {
    throw new Error('配置覆盖必须是合法的 JSON')
  }

  if (parsed == null || Array.isArray(parsed) || typeof parsed !== 'object') {
    throw new Error('配置覆盖必须是 JSON 对象')
  }

  return parsed as DatabaseConfig
}

async function handleSubmit() {
  if (!props.site) return
  saving.value = true
  error.value = ''

  try {
    const result = await registryStore.createTask(props.site.site_id, {
      task_name: taskName.value.trim() || undefined,
      task_type: taskType.value,
      priority: priority.value,
      config_override: parseConfigOverride(),
    })
    emit('created', {
      taskId: result.task_id,
      message: result.message,
      siteName: props.site.name,
    })
  } catch (err: unknown) {
    error.value = err instanceof Error ? err.message : '创建任务失败'
  } finally {
    saving.value = false
  }
}
</script>

<template>
  <Teleport to="body">
    <div v-if="open && site" class="fixed inset-0 z-50 flex items-center justify-center">
      <div class="absolute inset-0 bg-black/50" @click="emit('close')" />
      <div class="relative w-full max-w-2xl rounded-lg border border-border bg-background shadow-xl">
        <div class="flex items-center justify-between border-b border-border px-6 py-4">
          <div>
            <h3 class="text-lg font-semibold">创建注册表任务</h3>
            <p class="text-sm text-muted-foreground">{{ site.name }} · {{ site.site_id }}</p>
          </div>
          <button
            class="inline-flex h-8 w-8 items-center justify-center rounded-md hover:bg-accent transition-colors"
            @click="emit('close')"
          >
            <X class="h-4 w-4" />
          </button>
        </div>

        <div class="space-y-5 px-6 py-5">
          <div v-if="error" class="rounded-lg border border-destructive/50 bg-destructive/5 px-4 py-3 text-sm text-destructive">
            {{ error }}
          </div>

          <div class="grid gap-4 md:grid-cols-2">
            <label class="space-y-2 text-sm md:col-span-2">
              <span class="font-medium">任务名称</span>
              <input
                v-model="taskName"
                type="text"
                class="flex h-10 w-full rounded-md border border-input bg-transparent px-3 text-sm shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
              />
            </label>

            <label class="space-y-2 text-sm">
              <span class="font-medium">任务类型</span>
              <select
                v-model="taskType"
                class="flex h-10 w-full rounded-md border border-input bg-transparent px-3 text-sm shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
              >
                <option v-for="option in taskTypeOptions" :key="String(option)" :value="option">
                  {{ TASK_TYPE_LABELS[String(option)] || option }}
                </option>
              </select>
            </label>

            <label class="space-y-2 text-sm">
              <span class="font-medium">优先级</span>
              <select
                v-model="priority"
                class="flex h-10 w-full rounded-md border border-input bg-transparent px-3 text-sm shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
              >
                <option v-for="(label, key) in PRIORITY_LABELS" :key="key" :value="key">
                  {{ label }}
                </option>
              </select>
            </label>

            <label class="space-y-2 text-sm md:col-span-2">
              <span class="font-medium">配置覆盖</span>
              <textarea
                v-model="configOverrideText"
                rows="14"
                spellcheck="false"
                class="w-full rounded-md border border-input bg-transparent px-3 py-2 font-mono text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
              />
              <p class="text-xs text-muted-foreground">直接编辑当前注册表配置；留空则使用站点默认配置。</p>
            </label>
          </div>
        </div>

        <div class="flex items-center justify-end gap-3 border-t border-border px-6 py-4">
          <button
            type="button"
            class="inline-flex h-9 items-center rounded-md border border-input px-4 text-sm font-medium hover:bg-accent transition-colors"
            @click="emit('close')"
          >
            取消
          </button>
          <button
            type="button"
            :disabled="saving"
            class="inline-flex h-9 items-center rounded-md bg-primary px-4 text-sm font-medium text-primary-foreground shadow hover:bg-primary/90 disabled:opacity-60"
            @click="handleSubmit"
          >
            {{ saving ? '创建中...' : '创建任务' }}
          </button>
        </div>
      </div>
    </div>
  </Teleport>
</template>
