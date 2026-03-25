<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import { useDisplay } from 'vuetify';
import { useRoute, useRouter } from 'vue-router';
import {
  BookMarkedIcon,
  BoxesIcon,
  DatabaseZapIcon,
  FolderKanbanIcon,
  LayoutDashboardIcon,
  MonitorSmartphoneIcon,
  RocketIcon,
  ScanSearchIcon,
  Settings2Icon,
  SparklesIcon,
  WorkflowIcon,
  WrenchIcon,
} from 'lucide-vue-next';

import { appRoutes } from '@/router/routes';

const route = useRoute();
const router = useRouter();
const { mdAndDown } = useDisplay();

const iconMap = {
  dashboard: LayoutDashboardIcon,
  projects: FolderKanbanIcon,
  tasks: BoxesIcon,
  batch: BookMarkedIcon,
  deployment: RocketIcon,
  wizard: SparklesIcon,
  sync: WorkflowIcon,
  remote: ScanSearchIcon,
  incremental: WorkflowIcon,
  db: DatabaseZapIcon,
  spatial: DatabaseZapIcon,
  visualization: SparklesIcon,
  settings: Settings2Icon,
  connection: Settings2Icon,
  tools: WrenchIcon,
  tray: WrenchIcon,
  sctn: WrenchIcon,
  room: WrenchIcon,
  viewer: MonitorSmartphoneIcon,
} as const;

const drawer = ref(true);

watch(
  mdAndDown,
  (isMobile) => {
    drawer.value = !isMobile;
  },
  { immediate: true },
);

const groupedNav = computed(() => {
  const routes = appRoutes
    .filter((entry) => typeof entry.path === 'string' && entry.meta?.navGroup && entry.meta?.navLabel)
    .map((entry) => ({
      path: String(entry.path),
      group: String(entry.meta?.navGroup),
      label: String(entry.meta?.navLabel),
      order: Number(entry.meta?.navOrder || 0),
      icon: iconMap[entry.meta?.navIcon as keyof typeof iconMap] || LayoutDashboardIcon,
    }))
    .sort((a, b) => a.order - b.order);

  const groupOrder = ['Dashboard', 'Projects', 'Tasks', 'Deployment', 'Sync', 'DB', 'Settings', 'Tools', 'Viewer'];
  return groupOrder
    .map((group) => ({
      group,
      items: routes.filter((entry) => entry.group === group),
    }))
    .filter((entry) => entry.items.length > 0);
});

const primaryGroups = computed(() => groupedNav.value.filter((group) => group.group !== 'Viewer'));

const mobileNav = computed(() =>
  groupedNav.value.flatMap((group) => group.items).filter((item) =>
    ['/dashboard', '/projects', '/tasks', '/deployment/sites'].includes(item.path),
  ),
);

const currentTitle = computed(() => {
  const current = groupedNav.value
    .flatMap((group) => group.items)
    .find((item) => route.path === item.path || route.path.startsWith(`${item.path}/`));
  return current?.label || 'AIOS 控制台';
});

const currentSection = computed(() => {
  const current = groupedNav.value
    .flatMap((group) => group.items.map((item) => ({ ...item, group: group.group })))
    .find((item) => route.path === item.path || route.path.startsWith(`${item.path}/`));
  return current?.group || 'Console';
});

function navigateTo(path: string) {
  router.push(path);
  if (mdAndDown.value) {
    drawer.value = false;
  }
}
</script>

<template>
  <v-app class="console-app">
    <v-navigation-drawer
      :model-value="drawer"
      :permanent="!mdAndDown"
      :temporary="mdAndDown"
      width="308"
      class="console-drawer"
      @update:model-value="drawer = $event"
    >
      <div class="brand-block">
        <div class="brand-mark">A</div>
        <div>
          <p class="brand-title">AIOS Console</p>
          <p class="brand-subtitle">Vuetify dashboard shell for `/console/*`</p>
        </div>
      </div>

      <div class="drawer-summary">
        <p class="drawer-kicker">Foundation</p>
        <h2>One shell for legacy console migration</h2>
        <p>
          Keep every navigation target inside `/console/*`, with stable deep links for the next
          milestone wave.
        </p>
      </div>

      <div class="drawer-groups">
        <section v-for="group in primaryGroups" :key="group.group" class="drawer-group">
          <p class="drawer-group-title">{{ group.group }}</p>
          <v-list class="drawer-list" density="comfortable" nav>
            <v-list-item
              v-for="item in group.items"
              :key="item.path"
              :active="route.path === item.path || route.path.startsWith(`${item.path}/`)"
              rounded="xl"
              class="drawer-item"
              link
              @click="navigateTo(item.path)"
            >
              <template #prepend>
                <component :is="item.icon" class="shell-nav-icon" />
              </template>
              <v-list-item-title class="drawer-item-title">{{ item.label }}</v-list-item-title>
            </v-list-item>
          </v-list>
        </section>
      </div>

      <div class="viewer-entry">
        <button type="button" class="viewer-button" @click="navigateTo('/viewer/preview')">
          <MonitorSmartphoneIcon class="shell-nav-icon" />
          <span>Viewer / Preview placeholder</span>
        </button>
      </div>
    </v-navigation-drawer>

    <v-app-bar flat class="console-app-bar">
      <div class="app-bar-copy">
        <p class="shell-eyebrow">{{ currentSection }}</p>
        <h1>{{ currentTitle }}</h1>
      </div>
      <v-spacer />
      <div class="header-badge">Rust API + Vue Router history + Vuetify</div>
    </v-app-bar>

    <v-main class="console-main">
      <div class="shell-content">
        <slot />
      </div>
    </v-main>

    <nav class="shell-mobile-nav">
      <button
        v-for="item in mobileNav"
        :key="`${item.path}-mobile`"
        type="button"
        class="shell-mobile-item"
        :class="{ active: route.path === item.path || route.path.startsWith(`${item.path}/`) }"
        @click="navigateTo(item.path)"
      >
        <component :is="item.icon" class="shell-nav-icon" />
        <span>{{ item.label }}</span>
      </button>
    </nav>
  </v-app>
</template>
