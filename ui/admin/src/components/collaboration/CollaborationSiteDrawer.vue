<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { X } from 'lucide-vue-next'
import type { CollaborationSite, CreateCollaborationSiteRequest } from '@/types/collaboration'

const props = defineProps<{
  open: boolean
  site: CollaborationSite | null
  disabled?: boolean
  save: (payload: CreateCollaborationSiteRequest) => Promise<void>
}>()

const emit = defineEmits<{
  close: []
}>()

const saving = ref(false)
const error = ref('')

const form = ref({
  name: '',
  location: '',
  http_host: '',
  dbnums: '',
  notes: '',
})

const isEditing = computed(() => props.site != null)
const title = computed(() => isEditing.value ? '编辑协同站点' : '新建协同站点')
const inputClass = 'flex h-9 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-sm transition-colors placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring'
const textareaClass = 'flex min-h-[96px] w-full rounded-md border border-input bg-transparent px-3 py-2 text-sm shadow-sm transition-colors placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring'

watch(() => props.open, (open) => {
  if (!open) return

  error.value = ''
  form.value = {
    name: props.site?.name ?? '',
    location: props.site?.location ?? '',
    http_host: props.site?.http_host ?? '',
    dbnums: props.site?.dbnums ?? '',
    notes: props.site?.notes ?? '',
  }
})

const canSubmit = computed(() => {
  return String(form.value.name ?? '').trim() !== '' && !saving.value && props.disabled !== true
})

function trimOrNull(value: string | number | null | undefined) {
  const next = String(value ?? '').trim()
  return next === '' ? null : next
}

async function handleSubmit() {
  if (!canSubmit.value) return

  error.value = ''
  saving.value = true

  try {
    const payload: CreateCollaborationSiteRequest = {
      name: String(form.value.name ?? '').trim(),
      location: trimOrNull(form.value.location),
      http_host: trimOrNull(form.value.http_host),
      dbnums: trimOrNull(form.value.dbnums),
      notes: trimOrNull(form.value.notes),
    }

    await props.save(payload)
    emit('close')
  } catch (err: unknown) {
    error.value = err instanceof Error ? err.message : '保存协同站点失败'
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
        <div class="absolute right-0 top-0 flex h-full w-full max-w-[480px] flex-col border-l border-border bg-background shadow-xl">
          <div class="flex items-center justify-between border-b border-border px-6 py-4">
            <div>
              <h3 class="text-lg font-semibold">{{ title }}</h3>
              <p class="mt-1 text-sm text-muted-foreground">
                {{ isEditing ? '修改站点地址、区域和说明。' : '把新的协同站点加入当前协同组。' }}
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
              <label class="text-sm font-medium">站点名称 *</label>
              <input v-model="form.name" type="text" required placeholder="例：石家庄站点" :class="inputClass" />
            </div>

            <div class="space-y-2">
              <label class="text-sm font-medium">区域标识</label>
              <input v-model="form.location" type="text" placeholder="例：sjz" :class="inputClass" />
            </div>

            <div class="space-y-2">
              <label class="text-sm font-medium">HTTP Host</label>
              <input
                v-model="form.http_host"
                type="text"
                placeholder="http://host:port/assets/archives"
                :class="inputClass"
              />
            </div>

            <div class="space-y-2">
              <label class="text-sm font-medium">DBNums</label>
              <input v-model="form.dbnums" type="text" placeholder="7997,8001" :class="inputClass" />
            </div>

            <div class="space-y-2">
              <label class="text-sm font-medium">备注</label>
              <textarea v-model="form.notes" :class="textareaClass" placeholder="可填写站点说明或同步备注" />
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
