<script setup lang="ts">
import { onMounted } from 'vue'

onMounted(() => {
  window.location.replace('/admin/#/registry')
})
</script>

<template>
  <div class="page-stack">
    <section class="hero-panel slim placeholder-hero">
      <div>
        <p class="section-tag">Deployment</p>
        <h2>正在跳转到 Admin</h2>
        <p class="hero-copy">
          站点管理已经正式收口到 <code>/admin/#/registry</code>，这里仅保留兼容跳转。
        </p>
      </div>
    </section>
  </div>
</template>
