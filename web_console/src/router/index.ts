import { createRouter, createWebHistory } from 'vue-router';
import { appRoutes } from '@/router/routes';

const router = createRouter({
  history: createWebHistory('/console/'),
  routes: appRoutes,
});

router.afterEach((to) => {
  document.title = `${String(to.meta.title || 'AIOS 控制台')} · AIOS 控制台`;
});

export default router;
