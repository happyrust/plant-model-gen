<script setup lang="ts">
import { ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useAuthStore } from '@/stores/auth'

const route = useRoute()
const router = useRouter()
const auth = useAuthStore()

const username = ref('')
const password = ref('')
const rememberMe = ref(false)

async function handleSubmit() {
  try {
    await auth.login({ username: username.value, password: password.value })
    const redirect = typeof route.query.redirect === 'string' && route.query.redirect.startsWith('/')
      ? route.query.redirect
      : '/'
    router.replace(redirect)
  } catch {
    // error is handled by the store
  }
}
</script>

<template>
  <div class="flex min-h-screen items-center justify-center bg-background">
    <div class="mx-auto w-full max-w-sm space-y-6">
      <div class="space-y-2 text-center">
        <div class="mx-auto flex h-10 w-10 items-center justify-center rounded-lg bg-primary text-primary-foreground font-bold">P</div>
        <h1 class="text-2xl font-semibold tracking-tight">Plant Admin</h1>
        <p class="text-sm text-muted-foreground">登录以管理站点和任务</p>
      </div>
      <form class="space-y-4" @submit.prevent="handleSubmit">
        <div class="space-y-2">
          <label class="text-sm font-medium leading-none" for="username">用户名</label>
          <input
            id="username"
            v-model="username"
            type="text"
            placeholder="admin"
            required
            class="flex h-9 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-sm transition-colors placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
          />
        </div>
        <div class="space-y-2">
          <label class="text-sm font-medium leading-none" for="password">密码</label>
          <input
            id="password"
            v-model="password"
            type="password"
            required
            class="flex h-9 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-sm transition-colors placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
          />
        </div>
        <div class="flex items-center gap-2">
          <input
            id="remember"
            v-model="rememberMe"
            type="checkbox"
            class="h-4 w-4 rounded border border-input"
          />
          <label class="text-sm text-muted-foreground" for="remember">记住我</label>
        </div>
        <div v-if="auth.loginError" class="rounded-md border border-destructive/50 bg-destructive/10 p-3 text-sm text-destructive">
          {{ auth.loginError }}
        </div>
        <button
          type="submit"
          :disabled="auth.loading"
          class="inline-flex h-9 w-full items-center justify-center rounded-md bg-primary px-4 text-sm font-medium text-primary-foreground shadow transition-colors hover:bg-primary/90 disabled:pointer-events-none disabled:opacity-50"
        >
          {{ auth.loading ? '登录中...' : '登录' }}
        </button>
      </form>
    </div>
  </div>
</template>
