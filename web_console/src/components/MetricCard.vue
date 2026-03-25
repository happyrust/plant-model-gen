<script setup lang="ts">
import type { MetricCard } from '@/types/dashboard';

defineProps<{ card: MetricCard }>();
</script>

<template>
  <article class="metric-card">
    <p class="metric-label">{{ card.label }}</p>
    <p class="metric-value">{{ card.value }}</p>
    <p class="metric-hint">{{ card.hint }}</p>
  </article>
</template>
