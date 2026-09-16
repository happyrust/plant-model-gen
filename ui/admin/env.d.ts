/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly VITE_ENABLE_OFFLINE_DEPLOY?: string
}

interface ImportMeta {
  readonly env: ImportMetaEnv
}
