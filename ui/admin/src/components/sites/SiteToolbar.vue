<script setup lang="ts">
import { ref } from 'vue'
import { useRouter } from 'vue-router'
import { Search, Plus } from 'lucide-vue-next'

const router = useRouter()
const search = ref('')
const envFilter = ref('')
const statusFilter = ref('')

defineExpose({ search, envFilter, statusFilter })
</script>

<template>
  <div class="flex flex-wrap items-center gap-3">
    <div class="relative flex-1 min-w-[200px]">
      <Search class="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
      <input v-model="search" type="text" placeholder="搜索站点..."
        class="flex h-9 w-full rounded-md border border-input bg-transparent pl-9 pr-3 py-1 text-sm shadow-sm placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring" />
    </div>
    <select v-model="envFilter"
      class="flex h-9 rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring">
      <option value="">所有环境</option>
      <option value="dev">开发</option>
      <option value="staging">预发布</option>
      <option value="prod">生产</option>
    </select>
    <select v-model="statusFilter"
      class="flex h-9 rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring">
      <option value="">所有状态</option>
      <option value="running">运行中</option>
      <option value="stopped">已停止</option>
      <option value="error">异常</option>
      <option value="pending">待解析</option>
    </select>
    <button @click="router.push('/tasks/new')"
      class="inline-flex h-9 items-center gap-2 rounded-md bg-primary px-4 text-sm font-medium text-primary-foreground shadow hover:bg-primary/90 transition-colors">
      <Plus class="h-4 w-4" /> 新建站点
    </button>
  </div>
</template>
