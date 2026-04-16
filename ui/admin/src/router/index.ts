import { createRouter, createWebHashHistory } from 'vue-router'
import { useAuthStore } from '@/stores/auth'

function normalizeRedirectTarget(value: unknown) {
  if (typeof value !== 'string' || !value.startsWith('/')) {
    return '/'
  }
  return value
}

const router = createRouter({
  history: createWebHashHistory('/admin/'),
  routes: [
    {
      path: '/login',
      name: 'login',
      component: () => import('@/views/LoginView.vue'),
      meta: { guest: true },
    },
    {
      path: '/',
      component: () => import('@/components/layout/MainLayout.vue'),
      meta: { requiresAuth: true },
      children: [
        { path: '', redirect: '/sites' },
        {
          path: 'sites',
          name: 'sites',
          component: () => import('@/views/SitesView.vue'),
        },
        {
          path: 'registry',
          name: 'registry',
          component: () => import('@/views/RegistrySitesView.vue'),
        },
        {
          path: 'sites/:id',
          name: 'site-detail',
          component: () => import('@/views/SiteDetailView.vue'),
        },
        {
          path: 'tasks/new',
          name: 'task-wizard',
          component: () => import('@/views/TaskWizardView.vue'),
        },
        {
          path: 'tasks',
          name: 'tasks',
          component: () => import('@/views/TaskProgressView.vue'),
        },
        {
          path: 'tasks/:id',
          name: 'task-detail',
          component: () => import('@/views/TaskProgressView.vue'),
        },
        {
          path: 'collaboration',
          name: 'collaboration',
          component: () => import('@/views/CollaborationWorkbenchView.vue'),
        },
      ],
    },
  ],
})

router.beforeEach((to) => {
  const auth = useAuthStore()
  if (to.meta.requiresAuth && !auth.isAuthenticated) {
    return {
      name: 'login',
      query: {
        redirect: to.fullPath,
      },
    }
  }
  if (to.meta.guest && auth.isAuthenticated) {
    return { path: normalizeRedirectTarget(to.query.redirect) }
  }
})

export default router
