<script setup lang="ts">
import { onMounted } from 'vue'

onMounted(() => {
  const search = window.location.search || ''
  window.location.replace('/admin/#/collaboration' + search)
})
</script>

<template>
  <div class="page-stack">
    <section class="hero-panel slim placeholder-hero">
      <div>
        <p class="section-tag">Sync</p>
        <h2>正在跳转到 Admin</h2>
        <p class="hero-copy">
          异地协同已经正式收口到 <code>/admin/#/collaboration</code>，这里仅保留兼容跳转。
        </p>
      </div>
    </section>
  </div>
</template>
