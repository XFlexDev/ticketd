import vue from '@vitejs/plugin-vue'
import vuetify from 'vite-plugin-vuetify'
import { defineConfig } from 'vite'

export default defineConfig({
  plugins: [vue(), vuetify({ autoImport: true })],
  server: { host: '0.0.0.0', allowedHosts: true, proxy: { '/api': 'http://127.0.0.1:8477' } },
})
