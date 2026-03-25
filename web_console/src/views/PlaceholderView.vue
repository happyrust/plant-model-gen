<script setup lang="ts">
defineProps<{
  eyebrow: string;
  title: string;
  description: string;
  chips?: string[];
}>();
</script>

<template>
  <div class="page-stack">
    <section class="hero-panel slim placeholder-hero">
      <div>
        <p class="section-tag">{{ eyebrow }}</p>
        <h2>{{ title }}</h2>
        <p class="hero-copy">{{ description }}</p>
      </div>
    </section>

    <section class="content-grid single-column">
      <article class="panel-card placeholder-card">
        <div class="panel-head">
          <div>
            <p class="panel-eyebrow">Console foundation</p>
            <h3>Route is ready for deep-link validation</h3>
          </div>
        </div>
        <p class="placeholder-text">
          This page is intentionally a placeholder for the current milestone. It confirms the
          Vuetify shell, grouped navigation, and `/console/*` history fallback before the
          feature-specific UI is migrated.
        </p>
        <div v-if="chips?.length" class="chip-row">
          <span v-for="chip in chips" :key="chip" class="soft-chip">{{ chip }}</span>
        </div>
      </article>
    </section>
  </div>
</template>
