<template>
  <v-app>
    <v-app-bar flat border color="white">
      <v-app-bar-title class="font-weight-bold">
        <router-link to="/" style="text-decoration:none;color:inherit">ticketd</router-link>
      </v-app-bar-title>
      <v-spacer />
      <v-btn v-if="me" to="/servers" variant="text">Servers</v-btn>
      <v-btn v-if="!me" href="/api/auth/discord" color="primary" variant="flat" prepend-icon="mdi-discord">Login</v-btn>
      <v-menu v-else>
        <template #activator="{ props }"><v-btn v-bind="props" variant="text">{{ me.name }}</v-btn></template>
        <v-list density="compact"><v-list-item href="/api/logout" title="Log out" prepend-icon="mdi-logout" /></v-list>
      </v-menu>
    </v-app-bar>
    <v-main><v-container><router-view @loaded="onLoaded" /></v-container></v-main>
    <v-footer class="justify-center text-caption" color="white" border>© {{ new Date().getFullYear() }} ticketd — Kreatix</v-footer>
  </v-app>
</template>
<script setup>
import { ref, onMounted } from 'vue'
import { api } from './api'
const me = ref(null)
onMounted(async () => { try { const d = await api('/api/me'); me.value = d.user } catch {} })
function onLoaded(u) { me.value = u }
</script>
