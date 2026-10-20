import { createApp } from 'vue'
import { createVuetify } from 'vuetify'
import { aliases, mdi } from 'vuetify/iconsets/mdi-svg'
import { createRouter, createWebHistory } from 'vue-router'
import 'vuetify/styles'
import '@fontsource/roboto/latin-400.css'
import '@fontsource/roboto/latin-500.css'
import './style.css'
import App from './App.vue'
import Home from './views/Home.vue'
import Guild from './views/Guild.vue'
import Ticket from './views/Ticket.vue'

const router = createRouter({
  history: createWebHistory(),
  routes: [
    { path: '/', component: Home },
    { path: '/servers', redirect: '/' },
    { path: '/g/:id', component: Guild },
    { path: '/t/:id', component: Ticket },
    { path: '/leaderboard/:id', component: () => import('./views/Leaderboard.vue') },
  ],
  scrollBehavior: () => ({ top: 0 }),
})

const stored = localStorage.getItem('kt-theme')
const dark = stored !== 'light'

createApp(App).use(router).use(createVuetify({
  icons: { defaultSet: 'mdi', aliases, sets: { mdi } },
  theme: {
    defaultTheme: dark ? 'dark' : 'light',
    themes: {
      light: { colors: { primary: '#2e7d32', secondary: '#1976d2', background: '#ffffff', surface: '#ffffff' } },
      dark: { colors: { primary: '#66bb6a', 'on-primary': '#07190a', secondary: '#90caf9', background: '#000000', surface: '#101010', 'surface-variant': '#242424' } },
    },
  },
  defaults: {
    VTextField: { variant: 'filled', color: 'primary' },
    VSelect: { variant: 'filled', color: 'primary' },
    VBtn: { rounded: 'sm', height: 40 },
    VSwitch: { color: 'primary', inset: true, hideDetails: true },
  },
})).mount('#app')
