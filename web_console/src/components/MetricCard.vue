<script setup lang="ts">
import { computed } from 'vue';

import type { MetricCard } from '@/types/dashboard';

const props = defineProps<{ card: MetricCard }>();

const toneClass = computed(() => `metric-card tone-${props.card.tone || 'primary'}`);
</script>

<template>
  <article :class="toneClass">
    <div class="metric-topline">
      <p class="metric-label">{{ card.label }}</p>
      <span v-if="card.trend" class="metric-trend">{{ card.trend }}</span>
    </div>
    <p class="metric-value">{{ card.value }}</p>
    <p class="metric-hint">{{ card.hint }}</p>
  </article>
</template>
