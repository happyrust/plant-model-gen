<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { X } from 'lucide-vue-next'
import type { CollaborationEnv, CreateCollaborationEnvRequest } from '@/types/collaboration'

const props = defineProps<{
  open: boolean
  env: CollaborationEnv | null
  disabled?: boolean
  save: (payload: CreateCollaborationEnvRequest) => Promise<void>
}>()

const emit = defineEmits<{
  close: []
}>()

const saving = ref(false)
const error = ref('')

function onEscape(e: KeyboardEvent) {
  if (e.key === 'Escape' && props.open && !saving.value) emit('close')
}
onMounted(() => document.addEventListener('keydown', onEscape))
onUnmounted(() => document.removeEventListener('keydown', onEscape))

const form = ref({
  name: '',
  mqtt_host: '',
  mqtt_port: '',
  file_server_host: '',
  location: '',
  location_dbs: '',
  reconnect_initial_ms: '',
  reconnect_max_ms: '',
})

const isEditing = computed(() => props.env != null)
const title = computed(() => isEditing.value ? '编辑协同组' : '新建协同组')
const inputClass = 'flex h-9 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-sm transition-colors placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring'

watch(() => props.open, (open) => {
  if (!open) return

  error.value = ''
  form.value = {
    name: props.env?.name ?? '',
    mqtt_host: props.env?.mqtt_host ?? '',
    mqtt_port: props.env?.mqtt_port != null ? String(props.env.mqtt_port) : '',
    file_server_host: props.env?.file_server_host ?? '',
    location: props.env?.location ?? '',
    location_dbs: props.env?.location_dbs ?? '',
    reconnect_initial_ms: props.env?.reconnect_initial_ms != null ? String(props.env.reconnect_initial_ms) : '',
    reconnect_max_ms: props.env?.reconnect_max_ms != null ? String(props.env.reconnect_max_ms) : '',
  }
})

const canSubmit = computed(() => {
  return String(form.value.name ?? '').trim() !== '' && !saving.value && props.disabled !== true
})

function trimOrNull(value: string | number | null | undefined) {
  const next = String(value ?? '').trim()
  return next === '' ? null : next
}

function positiveIntOrNull(value: string | number | null | undefined, label: string) {
  const trimmed = String(value ?? '').trim()
  if (trimmed === '') return null

  const parsed = Number(trimmed)
  if (!Number.isInteger(parsed) || parsed <= 0) {
    throw new Error(`${label}必须为正整数`)
  }
  return parsed
}

async function handleSubmit() {
  if (!canSubmit.value) return

  error.value = ''
  saving.value = true

  try {
    const payload: CreateCollaborationEnvRequest = {
      name: String(form.value.name ?? '').trim(),
      mqtt_host: trimOrNull(form.value.mqtt_host),
      mqtt_port: positiveIntOrNull(form.value.mqtt_port, 'MQTT 端口'),
      file_server_host: trimOrNull(form.value.file_server_host),
      location: trimOrNull(form.value.location),
      location_dbs: trimOrNull(form.value.location_dbs),
      reconnect_initial_ms: positiveIntOrNull(form.value.reconnect_initial_ms, '重连初始间隔'),
      reconnect_max_ms: positiveIntOrNull(form.value.reconnect_max_ms, '重连最大间隔'),
    }

    await props.save(payload)
    emit('close')
  } catch (err: unknown) {
    error.value = err instanceof Error ? err.message : '保存协同组失败'
  } finally {
    saving.value = false
  }
}
</script>

<template>
  <Teleport to="body">
    <Transition name="drawer">
      <div v-if="open" class="fixed inset-0 z-50">
        <div class="absolute inset-0 bg-black/50" @click="!saving && emit('close')" />
        <div class="absolute right-0 top-0 flex h-full w-full max-w-[520px] flex-col border-l border-border bg-background shadow-xl">
          <div class="flex items-center justify-between border-b border-border px-6 py-4">
            <div>
              <h3 class="text-lg font-semibold">{{ title }}</h3>
              <p class="mt-1 text-sm text-muted-foreground">
                {{ isEditing ? '修改当前协同组的连接与区域配置。' : '创建新的协同组，保存后会自动切换到新协同组。' }}
              </p>
            </div>
            <button
              class="inline-flex h-8 w-8 items-center justify-center rounded-md transition-colors hover:bg-accent"
              :disabled="saving"
              @click="emit('close')"
            >
              <X class="h-4 w-4" />
            </button>
          </div>

          <form class="flex-1 space-y-4 overflow-auto px-6 py-4" @submit.prevent="handleSubmit">
            <div class="space-y-2">
              <label class="text-sm font-medium">协同组名称 *</label>
              <input v-model="form.name" type="text" required placeholder="例：华北主协同组" :class="inputClass" />
            </div>

            <div class="grid grid-cols-2 gap-4">
              <div class="space-y-2">
                <label class="text-sm font-medium">MQTT 主机</label>
                <input v-model="form.mqtt_host" type="text" placeholder="127.0.0.1" :class="inputClass" />
              </div>
              <div class="space-y-2">
                <label class="text-sm font-medium">MQTT 端口</label>
                <input v-model="form.mqtt_port" type="number" min="1" placeholder="1883" :class="inputClass" />
              </div>
            </div>

            <div class="space-y-2">
              <label class="text-sm font-medium">文件服务地址</label>
              <input
                v-model="form.file_server_host"
                type="text"
                placeholder="http://host:port/assets/archives"
                :class="inputClass"
              />
            </div>

            <div class="grid grid-cols-2 gap-4">
              <div class="space-y-2">
                <label class="text-sm font-medium">区域标识</label>
                <input v-model="form.location" type="text" placeholder="例：sjz" :class="inputClass" />
              </div>
              <div class="space-y-2">
                <label class="text-sm font-medium">本地区 DBNums</label>
                <input v-model="form.location_dbs" type="text" placeholder="7997,8001" :class="inputClass" />
              </div>
            </div>

            <div class="grid grid-cols-2 gap-4">
              <div class="space-y-2">
                <label class="text-sm font-medium">重连初始间隔(ms)</label>
                <input v-model="form.reconnect_initial_ms" type="number" min="1" placeholder="1000" :class="inputClass" />
              </div>
              <div class="space-y-2">
                <label class="text-sm font-medium">重连最大间隔(ms)</label>
                <input v-model="form.reconnect_max_ms" type="number" min="1" placeholder="30000" :class="inputClass" />
              </div>
            </div>

            <div class="rounded-md border border-border bg-muted/30 px-3 py-3 text-sm text-muted-foreground">
              当前版本仅保留旧页面已有字段，不新增说明字段。
            </div>

            <div
              v-if="error"
              class="rounded-md border border-destructive/50 bg-destructive/10 p-3 text-sm text-destructive"
            >
              {{ error }}
            </div>
          </form>

          <div class="flex justify-end gap-3 border-t border-border px-6 py-4">
            <button
              class="inline-flex h-9 items-center rounded-md border border-input bg-transparent px-4 text-sm font-medium shadow-sm transition-colors hover:bg-accent"
              :disabled="saving"
              @click="emit('close')"
            >
              取消
            </button>
            <button
              class="inline-flex h-9 items-center rounded-md bg-primary px-4 text-sm font-medium text-primary-foreground shadow transition-colors hover:bg-primary/90 disabled:pointer-events-none disabled:opacity-50"
              :disabled="!canSubmit"
              @click="handleSubmit"
            >
              {{ saving ? '保存中...' : '保存' }}
            </button>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
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
