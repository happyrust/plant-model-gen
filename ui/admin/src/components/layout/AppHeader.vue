<script setup lang="ts">
import { computed } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useAuthStore } from '@/stores/auth'
import { Server, ListTodo, LogOut, DatabaseBackup, Network } from 'lucide-vue-next'

const route = useRoute()
const router = useRouter()
const auth = useAuthStore()

const navItems = [
  { path: '/sites', label: '站点管理', icon: Server },
  { path: '/registry', label: '中心注册表', icon: DatabaseBackup },
  { path: '/collaboration', label: '异地协同', icon: Network },
  { path: '/tasks', label: '任务管理', icon: ListTodo },
]

const currentPath = computed(() => '/' + (route.path.split('/')[1] ?? ''))

async function handleLogout() {
  await auth.logout()
  router.push({ name: 'login' })
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
