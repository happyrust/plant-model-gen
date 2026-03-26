# Architecture

Architecture notes for the **web console (Vue SPA) migration**.

## High-level structure

- **Backend:** Rust `web_server` provides `/api/*` and serves the built SPA.
- **Frontend:** Vue 3 + Vite + Vuetify app under `web_console/`.

## Serving model

- SPA entry: `GET /console` and `GET /console/` return `web_console/dist/index.html`.
- SPA assets: served under `GET /console/assets/*` from `web_console/dist/assets`.
- History mode: unknown `/console/*` routes return SPA index (history fallback), while non-console prefixes (e.g. `/api/*`) must not be swallowed.

## Legacy console migration model

- Legacy HTML routes (e.g. `/tasks`, `/deployment-sites`, `/sync-control`, `/db-status`, ...) are migrated to SPA routes under `/console/*`.
- Legacy routes are finalized as **30x redirects** to their SPA equivalents (see validation contract mapping table).

