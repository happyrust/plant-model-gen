<script setup lang="ts">
import { ref, watch, computed } from 'vue'
import { useSitesStore } from '@/stores/sites'
import { sitesApi } from '@/api/sites'
import type { CreateManagedSiteRequest, ManagedProjectSite } from '@/types/site'
import { X } from 'lucide-vue-next'

const props = defineProps<{
  open: boolean
  siteId: string | null
}>()

const emit = defineEmits<{
  close: []
  saved: []
}>()

const sitesStore = useSitesStore()
const saving = ref(false)
const error = ref('')
const existingSite = ref<ManagedProjectSite | null>(null)

const form = ref<CreateManagedSiteRequest>({
  project_name: '',
  project_path: '',
  project_code: 0,
  manual_db_nums: [],
  db_port: 8020,
  web_port: 8080,
  bind_host: '0.0.0.0',
  db_user: 'root',
  db_password: 'root',
})

const manualDbNumsStr = ref('')

const isEditing = computed(() => !!props.siteId)
const title = computed(() => isEditing.value ? '编辑站点' : '新建站点')

watch(() => props.open, async (open) => {
  if (!open) return
  error.value = ''
  if (props.siteId) {
    try {
      existingSite.value = await sitesApi.get(props.siteId)
      const s = existingSite.value
      form.value = {
        project_name: s.project_name,
        project_path: s.project_path,
        project_code: s.project_code,
        manual_db_nums: s.manual_db_nums,
        db_port: s.db_port,
        web_port: s.web_port,
        bind_host: s.bind_host || '0.0.0.0',
      }
      manualDbNumsStr.value = s.manual_db_nums.join(', ')
    } catch (e) {
      error.value = e instanceof Error ? e.message : 'Failed to load site'
    }
  } else {
    existingSite.value = null
    form.value = {
      project_name: '',
      project_path: '',
      project_code: 0,
      manual_db_nums: [],
      db_port: 8020,
      web_port: 8080,
      bind_host: '0.0.0.0',
      db_user: 'root',
      db_password: 'root',
    }
    manualDbNumsStr.value = ''
  }
})

function parseDbNums() {
  form.value.manual_db_nums = manualDbNumsStr.value
    .split(/[,\s]+/)
    .map(Number)
    .filter((n) => !isNaN(n) && n > 0)
}

async function handleSubmit() {
  saving.value = true
  error.value = ''
  parseDbNums()
  try {
    if (isEditing.value && props.siteId) {
      await sitesStore.updateSite(props.siteId, form.value)
    } else {
      await sitesStore.createSite(form.value)
    }
    emit('saved')
  } catch (e) {
    error.value = e instanceof Error ? e.message : 'Save failed'
  } finally {
    saving.value = false
  }
}

const inputClass = 'flex h-9 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-sm transition-colors placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring'
</script>

<template>
  <Teleport to="body">
    <Transition name="drawer">
      <div v-if="open" class="fixed inset-0 z-50">
        <div class="absolute inset-0 bg-black/50" @click="emit('close')" />
        <div class="absolute right-0 top-0 h-full w-full max-w-[480px] bg-background border-l border-border shadow-xl flex flex-col">
          <!-- Header -->
          <div class="flex items-center justify-between border-b border-border px-6 py-4">
            <div>
              <h3 class="text-lg font-semibold">{{ title }}</h3>
              <div v-if="existingSite" class="mt-1">
                <span class="inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium"
                  :class="existingSite.status === 'Running' ? 'bg-green-100 text-green-800' :
                           existingSite.status === 'Failed' ? 'bg-red-100 text-red-800' :
                           'bg-muted text-muted-foreground'">
                  {{ existingSite.status }}
                </span>
              </div>
            </div>
            <button @click="emit('close')"
              class="inline-flex h-8 w-8 items-center justify-center rounded-md hover:bg-accent transition-colors">
              <X class="h-4 w-4" />
            </button>
          </div>

          <!-- Form -->
          <form class="flex-1 overflow-auto px-6 py-4 space-y-4" @submit.prevent="handleSubmit">
            <div class="space-y-2">
              <label class="text-sm font-medium">项目名称 *</label>
              <input v-model="form.project_name" type="text" required placeholder="例：AvevaMarineSample" :class="inputClass" />
            </div>

            <div class="space-y-2">
              <label class="text-sm font-medium">项目路径 *</label>
              <input v-model="form.project_path" type="text" required placeholder="/path/to/e3d_models" :class="inputClass" />
            </div>

            <div class="grid grid-cols-2 gap-4">
              <div class="space-y-2">
                <label class="text-sm font-medium">项目代码 *</label>
                <input v-model.number="form.project_code" type="number" required min="1" :class="inputClass" />
              </div>
              <div class="space-y-2">
                <label class="text-sm font-medium">绑定地址</label>
                <input v-model="form.bind_host" type="text" placeholder="0.0.0.0" :class="inputClass" />
              </div>
            </div>

            <div class="grid grid-cols-2 gap-4">
              <div class="space-y-2">
                <label class="text-sm font-medium">DB 端口 *</label>
                <input v-model.number="form.db_port" type="number" required min="1" max="65535" :class="inputClass" />
              </div>
              <div class="space-y-2">
                <label class="text-sm font-medium">Web 端口 *</label>
                <input v-model.number="form.web_port" type="number" required min="1" max="65535" :class="inputClass" />
              </div>
            </div>

            <div class="space-y-2">
              <label class="text-sm font-medium">手动 DB Nums <span class="text-muted-foreground">(可选，逗号分隔)</span></label>
              <input v-model="manualDbNumsStr" type="text" placeholder="7997, 7998, 7999" :class="inputClass" />
            </div>

            <div v-if="!isEditing" class="grid grid-cols-2 gap-4">
              <div class="space-y-2">
                <label class="text-sm font-medium">DB 用户名</label>
                <input v-model="form.db_user" type="text" placeholder="root" :class="inputClass" />
              </div>
              <div class="space-y-2">
                <label class="text-sm font-medium">DB 密码</label>
                <input v-model="form.db_password" type="password" placeholder="root" :class="inputClass" />
              </div>
            </div>

            <div v-if="error" class="rounded-md border border-destructive/50 bg-destructive/10 p-3 text-sm text-destructive">
              {{ error }}
            </div>
          </form>

          <!-- Footer -->
          <div class="border-t border-border px-6 py-4 flex justify-end gap-3">
            <button @click="emit('close')"
              class="inline-flex h-9 items-center rounded-md border border-input bg-transparent px-4 text-sm font-medium shadow-sm hover:bg-accent transition-colors">
              取消
            </button>
            <button @click="handleSubmit" :disabled="saving || !form.project_name || !form.project_path"
              class="inline-flex h-9 items-center rounded-md bg-primary px-4 text-sm font-medium text-primary-foreground shadow hover:bg-primary/90 transition-colors disabled:pointer-events-none disabled:opacity-50">
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
