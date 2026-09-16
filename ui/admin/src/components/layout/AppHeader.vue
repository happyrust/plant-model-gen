<script setup lang="ts">
import { computed, ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useAuthStore } from '@/stores/auth'
import { Server, ListTodo, LogOut, DatabaseBackup, Network, PackageOpen, Power, Table2 } from 'lucide-vue-next'
import { apiPost, extractErrorMessage } from '@/api/client'
import { OFFLINE_DEPLOY_ENABLED } from '@/lib/features'

interface QuickExitResponse {
  script_path: string
  port: number
}

const route = useRoute()
const router = useRouter()
const auth = useAuthStore()
const quickExiting = ref(false)

const navItems = computed(() => [
  { path: '/sites', label: '站点管理', icon: Server },
  ...(OFFLINE_DEPLOY_ENABLED ? [{ path: '/offline-deploy', label: '离线部署', icon: PackageOpen }] : []),
  { path: '/registry', label: '中心注册表', icon: DatabaseBackup },
  { path: '/data', label: '站点数据', icon: Table2 },
  { path: '/collaboration', label: '异地协同', icon: Network },
  { path: '/tasks', label: '任务管理', icon: ListTodo },
])

const currentPath = computed(() => '/' + (route.path.split('/')[1] ?? ''))

async function handleLogout() {
  await auth.logout()
  router.push({ name: 'login' })
}

async function handleQuickExit() {
  if (quickExiting.value) return
  const confirmed = window.confirm('将立即停止 Plant Admin、Nginx、SurrealDB 以及本部署目录相关端口进程。确定继续吗？')
  if (!confirmed) return
  quickExiting.value = true
  try {
    const result = await apiPost<QuickExitResponse>('/api/admin/system/quick-exit')
    window.alert(`快速退出命令已发送，将清理 admin 端口 ${result.port} 相关进程。`)
  } catch (error) {
    window.alert(`快速退出失败：${extractErrorMessage(error)}`)
    quickExiting.value = false
  }
}
</script>

<template>
  <header class="sticky top-0 z-50 w-full border-b border-border bg-background/95 backdrop-blur supports-[backdrop-filter]:bg-background/60">
    <div class="container mx-auto flex h-14 items-center px-6">
      <div class="mr-8 flex items-center gap-2 font-semibold">
        <div class="flex h-7 w-7 items-center justify-center rounded-md bg-primary text-primary-foreground text-xs font-bold">P</div>
        <span class="text-sm">Plant Admin</span>
      </div>
      <nav class="flex items-center gap-1">
        <router-link
          v-for="item in navItems"
          :key="item.path"
          :to="item.path"
          class="flex items-center gap-2 rounded-md px-3 py-1.5 text-sm font-medium transition-colors"
          :class="currentPath === item.path
            ? 'bg-accent text-accent-foreground'
            : 'text-muted-foreground hover:text-foreground hover:bg-accent/50'"
        >
          <component :is="item.icon" class="h-4 w-4" />
          {{ item.label }}
        </router-link>
      </nav>
      <div class="ml-auto flex items-center gap-3">
        <span class="text-sm text-muted-foreground">{{ auth.username }}</span>
        <button
          class="flex h-8 w-8 items-center justify-center rounded-md text-destructive transition-colors hover:bg-destructive/10 disabled:cursor-not-allowed disabled:opacity-60"
          :title="quickExiting ? '正在退出' : '快速退出 Plant Admin'"
          :disabled="quickExiting"
          @click="handleQuickExit"
        >
          <Power class="h-4 w-4" />
        </button>
        <button
          class="flex h-8 w-8 items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-foreground transition-colors"
          title="登出"
          @click="handleLogout"
        >
          <LogOut class="h-4 w-4" />
        </button>
      </div>
    </div>
  </header>
</template>
