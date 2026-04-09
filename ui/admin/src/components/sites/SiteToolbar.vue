<script setup lang="ts">
import { ref } from 'vue'
import { Search, Plus } from 'lucide-vue-next'

const search = ref('')
const statusFilter = ref('')

const emit = defineEmits<{
  openDrawer: []
  filter: [search: string, status: string]
}>()

function emitFilter() {
  emit('filter', search.value, statusFilter.value)
}
</script>

<template>
  <div class="flex flex-wrap items-center gap-3">
    <div class="relative flex-1 min-w-[200px]">
      <Search class="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
      <input v-model="search" type="text" placeholder="搜索项目名称..."
        @input="emitFilter"
        class="flex h-9 w-full rounded-md border border-input bg-transparent pl-9 pr-3 py-1 text-sm shadow-sm placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring" />
    </div>
    <select v-model="statusFilter" @change="emitFilter"
      class="flex h-9 rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring">
      <option value="">所有状态</option>
      <option value="Running">运行中</option>
      <option value="Stopped">已停止</option>
      <option value="Failed">失败</option>
      <option value="Draft">草稿</option>
      <option value="Parsed">已解析</option>
    </select>
    <button @click="emit('openDrawer')"
      class="inline-flex h-9 items-center gap-2 rounded-md bg-primary px-4 text-sm font-medium text-primary-foreground shadow hover:bg-primary/90 transition-colors">
      <Plus class="h-4 w-4" /> 新建站点
    </button>
  </div>
</template>
