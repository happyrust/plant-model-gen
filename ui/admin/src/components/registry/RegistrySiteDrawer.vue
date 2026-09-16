<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { X } from 'lucide-vue-next'

import { useRegistryStore } from '@/stores/registry'
import {
  buildRegistrySitePayload,
  createEmptyRegistryForm,
  createRegistryFormFromSite,
} from '@/lib/registry'

const props = defineProps<{
  open: boolean
  siteId: string | null
}>()

const emit = defineEmits<{
  close: []
  saved: []
}>()

const registryStore = useRegistryStore()

const form = ref(createEmptyRegistryForm())
const loading = ref(false)
const saving = ref(false)
const error = ref('')

const title = computed(() =>
  props.siteId ? '编辑中心注册表站点' : '新建中心注册表站点',
)

watch(
  () => [props.open, props.siteId] as const,
  async ([open, siteId]) => {
    if (!open) {
      return
    }

    error.value = ''
    form.value = createEmptyRegistryForm()

    if (!siteId) {
      return
    }

    loading.value = true
    try {
      const site = await registryStore.fetchSite(siteId)
      form.value = createRegistryFormFromSite(site)
    } catch (err: unknown) {
      error.value = err instanceof Error ? err.message : '加载站点详情失败'
    } finally {
      loading.value = false
    }
  },
  { immediate: true },
)

async function handleSubmit() {
  saving.value = true
  error.value = ''

  try {
    const payload = buildRegistrySitePayload(form.value)
    if (props.siteId) {
      await registryStore.updateSite(props.siteId, payload)
    } else {
      await registryStore.createSite(payload)
    }
    emit('saved')
  } catch (err: unknown) {
    error.value = err instanceof Error ? err.message : '保存站点失败'
  } finally {
    saving.value = false
  }
}
</script>

<template>
  <Teleport to="body">
    <Transition name="drawer">
      <div v-if="open" class="fixed inset-0 z-50">
        <div class="absolute inset-0 bg-black/50" @click="emit('close')" />
        <div class="absolute right-0 top-0 flex h-full w-full max-w-[680px] flex-col border-l border-border bg-background shadow-xl">
          <div class="flex items-center justify-between border-b border-border px-6 py-4">
            <div>
              <h3 class="text-lg font-semibold">{{ title }}</h3>
              <p class="text-sm text-muted-foreground">统一维护中心注册表记录与基础配置</p>
            </div>
            <button
              class="inline-flex h-8 w-8 items-center justify-center rounded-md hover:bg-accent transition-colors"
              @click="emit('close')"
            >
              <X class="h-4 w-4" />
            </button>
          </div>

          <div class="flex-1 overflow-auto px-6 py-5">
            <div v-if="loading" class="py-16 text-center text-sm text-muted-foreground">
              正在加载站点详情...
            </div>

            <form v-else class="space-y-5" @submit.prevent="handleSubmit">
              <div
                v-if="error"
                class="rounded-lg border border-destructive/40 bg-destructive/5 px-4 py-3 text-sm text-destructive"
              >
                {{ error }}
              </div>

              <div class="grid gap-4 md:grid-cols-2">
                <label class="space-y-2 text-sm">
                  <span class="font-medium">站点 ID</span>
                  <input
                    v-model="form.site_id"
                    type="text"
                    placeholder="留空时按项目名和端口生成"
                    class="flex h-10 w-full rounded-md border border-input bg-transparent px-3 text-sm shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                  />
                </label>

                <label class="space-y-2 text-sm">
                  <span class="font-medium">站点名称</span>
                  <input
                    v-model="form.name"
                    type="text"
                    class="flex h-10 w-full rounded-md border border-input bg-transparent px-3 text-sm shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                  />
                </label>

                <label class="space-y-2 text-sm">
                  <span class="font-medium">项目名称</span>
                  <input
                    v-model="form.project_name"
                    type="text"
                    class="flex h-10 w-full rounded-md border border-input bg-transparent px-3 text-sm shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                  />
                </label>

                <label class="space-y-2 text-sm">
                  <span class="font-medium">项目代号</span>
                  <input
                    v-model.number="form.project_code"
                    type="number"
                    min="1"
                    class="flex h-10 w-full rounded-md border border-input bg-transparent px-3 text-sm shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                  />
                </label>

                <label class="space-y-2 text-sm md:col-span-2">
                  <span class="font-medium">项目路径</span>
                  <input
                    v-model="form.project_path"
                    type="text"
                    class="flex h-10 w-full rounded-md border border-input bg-transparent px-3 text-sm shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                  />
                </label>

                <label class="space-y-2 text-sm">
                  <span class="font-medium">前端地址</span>
                  <input
                    v-model="form.frontend_url"
                    type="text"
                    class="flex h-10 w-full rounded-md border border-input bg-transparent px-3 text-sm shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                  />
                </label>

                <label class="space-y-2 text-sm">
                  <span class="font-medium">后端地址</span>
                  <input
                    v-model="form.backend_url"
                    type="text"
                    class="flex h-10 w-full rounded-md border border-input bg-transparent px-3 text-sm shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                  />
                </label>

                <label class="space-y-2 text-sm">
                  <span class="font-medium">监听 Host</span>
                  <input
                    v-model="form.bind_host"
                    type="text"
                    class="flex h-10 w-full rounded-md border border-input bg-transparent px-3 text-sm shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                  />
                </label>

                <label class="space-y-2 text-sm">
                  <span class="font-medium">监听 Port</span>
                  <input
                    v-model.number="form.bind_port"
                    type="number"
                    min="1"
                    max="65535"
                    class="flex h-10 w-full rounded-md border border-input bg-transparent px-3 text-sm shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                  />
                </label>

                <label class="space-y-2 text-sm">
                  <span class="font-medium">区域</span>
                  <input
                    v-model="form.region"
                    type="text"
                    class="flex h-10 w-full rounded-md border border-input bg-transparent px-3 text-sm shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                  />
                </label>

                <label class="space-y-2 text-sm">
                  <span class="font-medium">环境</span>
                  <input
                    v-model="form.env"
                    type="text"
                    placeholder="prod / staging / dev"
                    class="flex h-10 w-full rounded-md border border-input bg-transparent px-3 text-sm shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                  />
                </label>

                <label class="space-y-2 text-sm">
                  <span class="font-medium">负责人</span>
                  <input
                    v-model="form.owner"
                    type="text"
                    class="flex h-10 w-full rounded-md border border-input bg-transparent px-3 text-sm shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                  />
                </label>

                <label class="space-y-2 text-sm">
                  <span class="font-medium">健康检查地址</span>
                  <input
                    v-model="form.health_url"
                    type="text"
                    class="flex h-10 w-full rounded-md border border-input bg-transparent px-3 text-sm shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                  />
                </label>

                <label class="space-y-2 text-sm md:col-span-2">
                  <span class="font-medium">描述</span>
                  <textarea
                    v-model="form.description"
                    rows="2"
                    class="w-full rounded-md border border-input bg-transparent px-3 py-2 text-sm shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                  />
                </label>

                <label class="space-y-2 text-sm md:col-span-2">
                  <span class="font-medium">备注</span>
                  <textarea
                    v-model="form.notes"
                    rows="3"
                    class="w-full rounded-md border border-input bg-transparent px-3 py-2 text-sm shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                  />
                </label>

                <label class="space-y-2 text-sm md:col-span-2">
                  <span class="font-medium">高级配置 JSON</span>
                  <textarea
                    v-model="form.config_json"
                    rows="14"
                    spellcheck="false"
                    class="w-full rounded-md border border-input bg-transparent px-3 py-2 font-mono text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                  />
                </label>
              </div>
            </form>
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
              {{ saving ? '保存中...' : '保存站点' }}
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
  transition: opacity 0.3s ease;
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
