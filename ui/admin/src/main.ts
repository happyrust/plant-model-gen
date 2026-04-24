import { createApp } from 'vue'
import { createPinia } from 'pinia'
import router from './router'
import App from './App.vue'
import { loadAppConfig } from '@/lib/app-config'
import './style.css'

const app = createApp(App)
app.use(createPinia())
app.use(router)

// 预拉 admin app-config（含 viewer_base_url）：fire-and-forget，不阻塞挂载。
// - 已登录：配置加载完成后，`resolveViewerBaseUrl` 通过 ref 的响应式自然刷新
//   当前页面上的 Viewer 按钮
// - 未登录：会 401 并被 client.onResponseError 重定向到 /login，配置留空，
//   等用户登录后进入需要 viewer 的视图时再手动 `loadAppConfig()`（可选）
loadAppConfig()

app.mount('#app')
