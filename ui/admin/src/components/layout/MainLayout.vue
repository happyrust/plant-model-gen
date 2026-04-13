<script setup lang="ts">
import { onMounted } from 'vue'
import { RouterView } from 'vue-router'
import { useAuthStore } from '@/stores/auth'
import AppHeader from './AppHeader.vue'

const auth = useAuthStore()

onMounted(() => {
  if (auth.isAuthenticated && !auth.username) {
    void auth.fetchMe()
  }
})
</script>

<template>
  <div class="min-h-screen bg-background">
    <AppHeader />
    <main class="container mx-auto px-6 py-6">
      <RouterView />
    </main>
  </div>
</template>
