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

type NavItem = {
  path: string;
  group: string;
  label: string;
  order: number;
  icon: (typeof iconMap)[keyof typeof iconMap];
  ariaLabel: string;
  itemId: string;
};

type NavGroup = {
  group: string;
  items: NavItem[];
  headingId: string;
  mobileHeadingId: string;
};

watch(
  mdAndDown,
  (isMobile) => {
    drawer.value = !isMobile;
  },
  { immediate: true },
);

const groupedNav = computed<NavGroup[]>(() => {
  const routes = appRoutes
    .filter((entry) => typeof entry.path === 'string' && entry.meta?.navGroup && entry.meta?.navLabel)
    .map((entry) => ({
      path: String(entry.path),
      group: String(entry.meta?.navGroup),
      label: String(entry.meta?.navLabel),
      order: Number(entry.meta?.navOrder || 0),
      icon: iconMap[entry.meta?.navIcon as keyof typeof iconMap] || LayoutDashboardIcon,
      ariaLabel: `${String(entry.meta?.navGroup)} - ${String(entry.meta?.navLabel)}`,
      itemId: String(entry.name || entry.path).replace(/[^a-zA-Z0-9_-]/g, '-'),
    }))
    .sort((a, b) => a.order - b.order);

  const groupOrder = ['Dashboard', 'Projects', 'Tasks', 'Deployment', 'Sync', 'DB', 'Settings', 'Tools', 'Viewer'];
  return groupOrder
    .map((group) => ({
      group,
      items: routes.filter((entry) => entry.group === group),
      headingId: `console-nav-group-${group.toLowerCase()}`,
      mobileHeadingId: `console-mobile-nav-group-${group.toLowerCase()}`,
    }))
    .filter((entry) => entry.items.length > 0);
});

const primaryGroups = computed(() => groupedNav.value);

const mobileNavPaths = ['/dashboard', '/projects', '/tasks', '/deployment/sites', '/viewer/preview'];

const mobileGroups = computed(() =>
  groupedNav.value
    .map((group) => ({
      ...group,
      items: group.items.filter((item) => mobileNavPaths.includes(item.path)),
    }))
    .filter((group) => group.items.length > 0),
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

      <nav class="drawer-groups" aria-label="Console navigation">
        <section v-for="group in primaryGroups" :key="group.group" class="drawer-group">
          <p :id="group.headingId" class="drawer-group-title">{{ group.group }}</p>
          <v-list
            class="drawer-list"
            density="comfortable"
            nav
            :aria-labelledby="group.headingId"
          >
            <v-list-item
              v-for="item in group.items"
              :key="item.path"
              :active="route.path === item.path || route.path.startsWith(`${item.path}/`)"
              rounded="xl"
              class="drawer-item"
              :title="item.ariaLabel"
              :aria-label="item.ariaLabel"
              :data-nav-item="item.itemId"
              tag="a"
              :href="`/console${item.path}`"
              @click.prevent="navigateTo(item.path)"
            >
              <template #prepend>
                <component :is="item.icon" class="shell-nav-icon" aria-hidden="true" />
              </template>
              <v-list-item-title class="drawer-item-title">{{ item.label }}</v-list-item-title>
            </v-list-item>
          </v-list>
        </section>
      </nav>

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

    <nav class="shell-mobile-nav" aria-label="Console quick navigation">
      <section v-for="group in mobileGroups" :key="`${group.group}-mobile`" class="shell-mobile-group">
        <p :id="group.mobileHeadingId" class="shell-mobile-group-title">{{ group.group }}</p>
        <div class="shell-mobile-grid" :aria-labelledby="group.mobileHeadingId">
          <button
            v-for="item in group.items"
            :key="`${item.path}-mobile`"
            type="button"
            class="shell-mobile-item"
            :class="{ active: route.path === item.path || route.path.startsWith(`${item.path}/`) }"
            :aria-label="item.ariaLabel"
            :data-nav-item="`${item.itemId}-mobile`"
            @click="navigateTo(item.path)"
          >
            <component :is="item.icon" class="shell-nav-icon" aria-hidden="true" />
            <span>{{ item.label }}</span>
          </button>
        </div>
      </section>
    </nav>
  </v-app>
</template>
