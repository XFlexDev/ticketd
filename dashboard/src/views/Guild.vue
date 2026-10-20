<script setup>
import { computed, onMounted, ref } from 'vue'
import { useRoute } from 'vue-router'
import { mdiArrowLeft, mdiChevronRight } from '@mdi/js'
import { api } from '../api'

const route = useRoute()
const id = route.params.id
const tab = ref(route.query.tab === 'settings' ? 'settings' : 'tickets')
const guild = ref(null)
const tickets = ref(null)
const error = ref('')
const s = ref({ ticket_category_id: null, mod_role_id: null, log_channel_id: null, ai_enabled: true })
const saved = ref(null)
const saving = ref(false)
const toast = ref({ show: false, text: '' })

const categories = computed(() => (guild.value?.channels || []).filter(c => c.type === 4).map(c => ({ title: c.name, value: c.id })))
const textChannels = computed(() => (guild.value?.channels || []).filter(c => c.type === 0 || c.type === 5).map(c => ({ title: `#${c.name}`, value: c.id })))
const roles = computed(() => (guild.value?.roles || []).map(r => ({ title: r.name, value: r.id })))
const dirty = computed(() => JSON.stringify(s.value) !== saved.value)
const openCount = computed(() => (tickets.value || []).filter(t => t.status === 'open').length)

onMounted(async () => {
  try {
    const [g, t] = await Promise.all([api(`/api/guilds/${id}`), api(`/api/guilds/${id}/tickets`)])
    guild.value = g
    tickets.value = t.tickets
    if (g.settings) s.value = { ...s.value, ...g.settings, ticket_category_id: g.settings.ticket_category_id === '0' ? null : g.settings.ticket_category_id, mod_role_id: g.settings.mod_role_id === '0' ? null : g.settings.mod_role_id }
    saved.value = JSON.stringify(s.value)
  } catch (e) { error.value = e.message }
})

async function save() {
  saving.value = true
  try {
    await api(`/api/guilds/${id}/settings`, { method: 'PUT', body: JSON.stringify(s.value) })
    saved.value = JSON.stringify(s.value)
    toast.value = { show: true, text: 'Settings saved' }
  } catch (e) { toast.value = { show: true, text: `Couldn't save: ${e.message}` } }
  saving.value = false
}
</script>

<template>
  <div class="workspace narrow">
    <nav class="back-navigation"><v-btn to="/" variant="text" :prepend-icon="mdiArrowLeft">All servers</v-btn></nav>
    <div class="workspace-heading">
      <div style="min-width:0">
        <h1>{{ guild?.name || (guild ? id : '\u00a0') }}</h1>
        <p class="muted small mono id-line">{{ id }}</p>
      </div>
    </div>
    <p v-if="error" class="muted">{{ error }}</p>
    <template v-else>
      <v-tabs v-model="tab" color="primary" class="workspace-tabs">
        <v-tab value="tickets">Tickets</v-tab>
        <v-tab value="settings">Settings</v-tab>
      </v-tabs>
      <v-window v-model="tab">
        <v-window-item value="tickets">
          <div v-if="!tickets" class="row-list"><div v-for="i in 3" :key="i" class="skeleton-row"><span /></div></div>
          <div v-else-if="!tickets.length" class="empty-state">
            <h2>No tickets yet</h2>
            <p class="muted">Run <code>/panel</code> in a channel to post the open-ticket button.</p>
          </div>
          <template v-else>
            <p class="muted small" style="margin-bottom:8px">{{ openCount }} open · {{ tickets.length }} total</p>
            <router-link v-for="t in tickets" :key="t.id" :to="`/t/${t.id}`" class="file-row" style="color:inherit;text-decoration:none">
              <div class="file-main">
                <span class="mono muted">#{{ t.id }}</span>
                <span class="ticket-subject">{{ t.subject || 'No subject' }}</span>
              </div>
              <span :class="['status', t.status]">{{ t.status }}</span>
              <v-icon :icon="mdiChevronRight" size="20" class="muted" />
            </router-link>
          </template>
        </v-window-item>
        <v-window-item value="settings">
          <div class="settings-form">
            <v-select v-model="s.ticket_category_id" :items="categories" label="Ticket category" :loading="!guild" hint="New ticket channels go here" persistent-hint class="field-block" />
            <v-select v-model="s.mod_role_id" :items="roles" label="Staff role" :loading="!guild" hint="Can see and answer every ticket" persistent-hint class="field-block" />
            <v-select v-model="s.log_channel_id" :items="textChannels" label="Log channel" clearable :loading="!guild" hint="Summaries and transcripts are posted here. Optional" persistent-hint class="field-block" />
            <div class="switch-row">
              <div><h3>AI replies</h3><p class="muted small">Answers members until staff joins</p></div>
              <v-switch v-model="s.ai_enabled" aria-label="AI replies" />
            </div>
            <div class="action-row">
              <v-btn color="primary" :loading="saving" :disabled="!dirty || !s.ticket_category_id || !s.mod_role_id" @click="save">Save</v-btn>
              <span v-if="dirty" class="muted small">Unsaved changes</span>
            </div>
          </div>
        </v-window-item>
      </v-window>
    </template>
    <v-snackbar v-model="toast.show" :timeout="2500">{{ toast.text }}</v-snackbar>
  </div>
</template>
