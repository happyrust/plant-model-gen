import path from 'node:path'
import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import tailwindcss from '@tailwindcss/vite'

const adminApiProxyTarget = process.env.ADMIN_API_PROXY_TARGET ?? 'http://localhost:8020'

export default defineConfig({
  base: '/admin/static/',
  plugins: [vue(), tailwindcss()],
  resolve: {
    alias: {
      '@': path.resolve(__dirname, './src'),
    },
  },
  build: {
    outDir: '../../src/web_server/static/admin',
    emptyOutDir: true,
  },
  server: {
    port: 5173,
    proxy: {
      '/api': adminApiProxyTarget,
    },
  },
})
