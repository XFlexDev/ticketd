<script setup>
import AdSlot from '../components/AdSlot.vue'
import { computed } from 'vue'
import { mdiChevronRight } from '@mdi/js'
import { session, inviteUrl, guildIcon } from '../api'
const initials = n => (n || '?').split(/\s+/).map(w => w[0]).join('').slice(0, 2).toUpperCase()
const guilds = computed(() => [...session.guilds].sort((a, b) => (a.name || '').localeCompare(b.name || '')))
</script>

<template>
  <div class="workspace narrow">
    <template v-if="!session.loaded">
      <div class="workspace-heading"><h1>Servers</h1></div>
      <div class="row-list"><div v-for="i in 3" :key="i" class="skeleton-row"><span /></div></div>
    </template>

    <div v-else-if="!session.user" class="onboarding">
      <h1>Tickets that answer themselves</h1>
      <p class="muted page-intro">A free Discord ticket bot. Members open a private ticket, AI replies right away, your staff takes over when it matters. Every ticket closes with a summary and a transcript.</p>
      <div class="action-row">
        <v-btn color="primary" href="/api/auth/discord">Sign in with Discord</v-btn>
        <a :href="inviteUrl" target="_blank" rel="noopener">Add to a server</a>
      </div>
    </div>

    <template v-else>
      <div class="workspace-heading">
        <h1>Servers</h1>
        <v-btn color="primary" :href="inviteUrl" target="_blank" rel="noopener">Add server</v-btn>
      </div>
      <div v-if="!guilds.length" class="empty-state">
        <h2>No servers yet</h2>
        <p class="muted">Servers show up here when the bot is in them and you have Manage Server. Added it just now? Sign in again.</p>
        <div class="action-row" style="justify-content:center">
          <v-btn color="primary" :href="inviteUrl" target="_blank" rel="noopener">Add to Discord</v-btn>
          <a href="/api/auth/discord">Sign in again</a>
        </div>
      </div>
      <div v-else class="row-list">
        <router-link v-for="g in guilds" :key="g.id" :to="`/g/${g.id}`" class="nav-row">
          <div class="row-lead">
            <div class="guild-avatar"><img v-if="guildIcon(g)" :src="guildIcon(g, 80)" alt="" loading="lazy"><template v-else>{{ initials(g.name) }}</template></div>
            <div><h2>{{ g.name || g.id }}</h2><p class="muted small mono">{{ g.id }}</p></div>
          </div>
          <span class="row-open">Open <v-icon :icon="mdiChevronRight" size="20" /></span>
        </router-link>
      </div>
    </template>
    <AdSlot />
  </div>
</template>
