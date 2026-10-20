<script setup>
import { computed, onMounted } from 'vue'
import { useTheme, useDisplay } from 'vuetify'
import { mdiDotsVertical, mdiLogout, mdiWeatherNight, mdiWhiteBalanceSunny } from '@mdi/js'
import { session, loadSession, inviteUrl } from './api'

const theme = useTheme()
const { mobile } = useDisplay({ mobileBreakpoint: 960 })
const isDark = computed(() => theme.global.name.value === 'dark')
function toggleTheme() {
  const next = isDark.value ? 'light' : 'dark'
  theme.global.name.value = next
  document.documentElement.style.background = next === 'dark' ? '#000' : '#fff'
  localStorage.setItem('kt-theme', next)
}
onMounted(() => loadSession())
</script>

<template>
  <v-app>
    <v-app-bar flat border="b" height="64">
      <v-app-bar-title class="brand">
        <router-link to="/" class="brand-link">Kev</router-link>
      </v-app-bar-title>
      <template v-if="!mobile">
        <v-btn to="/" variant="text" exact>Servers</v-btn>
        <v-btn :href="inviteUrl" target="_blank" rel="noopener" variant="text">Add to Discord</v-btn>
        <v-btn :icon="isDark ? mdiWhiteBalanceSunny : mdiWeatherNight" variant="text" aria-label="Toggle theme" @click="toggleTheme" />
        <v-btn v-if="session.user" :icon="mdiLogout" variant="text" href="/api/logout" aria-label="Sign out" />
      </template>
      <v-menu v-else>
        <template #activator="{ props }"><v-btn v-bind="props" :icon="mdiDotsVertical" variant="text" aria-label="More options" /></template>
        <v-list min-width="200">
          <v-list-item to="/" title="Servers" />
          <v-list-item :href="inviteUrl" target="_blank" title="Add to Discord" />
          <v-list-item :title="isDark ? 'Light theme' : 'Dark theme'" @click="toggleTheme" />
          <template v-if="session.user">
            <v-divider />
            <v-list-item href="/api/logout" title="Sign out" />
          </template>
        </v-list>
      </v-menu>
    </v-app-bar>
    <v-main>
      <router-view v-slot="{ Component, route }">
        <transition name="pane" mode="out-in"><component :is="Component" :key="route.fullPath" /></transition>
      </router-view>
      <footer class="site-foot muted small"><router-link to="/terms">Terms</router-link><router-link to="/privacy">Privacy</router-link></footer>
    </v-main>
  </v-app>
</template>

<style>
.site-foot { display: flex; justify-content: center; gap: 20px; padding: 28px 0 36px }
.site-foot a { color: inherit; text-decoration: none }
.site-foot a:hover { text-decoration: underline }
</style>
