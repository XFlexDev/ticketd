import { createApp } from 'vue'
import { createVuetify } from 'vuetify'
import 'vuetify/styles'
import '@mdi/font/css/materialdesignicons.min.css'
import * as components from 'vuetify/components'
import * as directives from 'vuetify/directives'
import { createRouter, createWebHistory } from 'vue-router'
import App from './App.vue'
import Home from './views/Home.vue'
import Servers from './views/Servers.vue'
import Guild from './views/Guild.vue'
import Ticket from './views/Ticket.vue'

const router = createRouter({
  history: createWebHistory(),
  routes: [
    { path: '/', component: Home },
    { path: '/servers', component: Servers },
    { path: '/g/:id', component: Guild },
    { path: '/t/:id', component: Ticket },
  ],
})

const vuetify = createVuetify({
  components,
  directives,
  theme: { defaultTheme: 'light' },
})

createApp(App).use(vuetify).use(router).mount('#app')
