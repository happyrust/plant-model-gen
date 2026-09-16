<script setup lang="ts">
import { computed } from 'vue';
import { useRoute } from 'vue-router';

const route = useRoute();
const projectId = computed(() => String(route.params.projectId || 'unknown'));
</script>

<template>
  <div class="page-stack">
    <section class="hero-panel slim placeholder-hero">
      <div>
        <p class="section-tag">Projects</p>
        <h2>Project workspace placeholder</h2>
        <p class="hero-copy">
          Project route `{{ projectId }}` is reserved so the Projects page can navigate deeper
          into the console without leaving the SPA shell.
        </p>
      </div>
    </section>

    <section class="content-grid single-column">
      <article class="panel-card placeholder-card">
        <div class="panel-head">
          <div>
            <p class="panel-eyebrow">Reserved route</p>
            <h3>Viewer and project-specific tools will attach here later</h3>
          </div>
        </div>
        <p class="placeholder-text">
          The route exists now to support deep links and validation. Subsequent milestones can
          replace this placeholder without changing the routing contract.
        </p>
      </article>
    </section>
  </div>
</template>
