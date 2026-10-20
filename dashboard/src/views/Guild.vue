<script setup>
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { useRoute } from 'vue-router'
import { mdiArrowLeft, mdiChevronRight } from '@mdi/js'
import { api } from '../api'
import Agent from './Agent.vue'
import Panels from './Panels.vue'
import Stats from './Stats.vue'

const route = useRoute()
const id = route.params.id
const tab = ref(['settings', 'agent', 'panels', 'stats'].includes(route.query.tab) ? route.query.tab : 'tickets')
const filter = ref('open')
const guild = ref(null)
const tickets = ref(null)
const error = ref('')
const s = ref({ ticket_category_id: null, mod_role_id: null, log_channel_id: null, ai_enabled: true, autoclose_hours: 0, close_on_leave: true, ratings: true })
const autoclose = [{ title: 'Off', value: 0 }, { title: '6 hours', value: 6 }, { title: '12 hours', value: 12 }, { title: '1 day', value: 24 }, { title: '2 days', value: 48 }, { title: '3 days', value: 72 }, { title: '1 week', value: 168 }, { title: '2 weeks', value: 336 }, { title: '30 days', value: 720 }]
const prioRank = { urgent: 0, high: 1, medium: 2, low: 3, '': 4 }
const saved = ref(null)
const saving = ref(false)
const toast = ref({ show: false, text: '' })

const categories = computed(() => (guild.value?.channels || []).filter(c => c.type === 4).map(c => ({ title: c.name, value: c.id })))
const textChannels = computed(() => (guild.value?.channels || []).filter(c => c.type === 0 || c.type === 5).map(c => ({ title: `#${c.name}`, value: c.id })))
const roles = computed(() => (guild.value?.roles || []).map(r => ({ title: r.name, value: r.id })))
const dirty = computed(() => JSON.stringify(s.value) !== saved.value)
const openCount = computed(() => (tickets.value || []).filter(t => t.status === 'open').length)
const shown = computed(() => {
  const list = (tickets.value || []).filter(t => filter.value === 'all' || t.status === filter.value)
  return filter.value === 'open' ? [...list].sort((a, b) => (prioRank[a.priority] ?? 4) - (prioRank[b.priority] ?? 4) || b.id - a.id) : list
})
function ago(sec) {
  const d = Date.now() / 1000 - sec
  if (d < 3600) return `${Math.max(1, Math.round(d / 60))}m`
  if (d < 86400) return `${Math.round(d / 3600)}h`
  return `${Math.round(d / 86400)}d`
}

onMounted(async () => {
  try {
    const [g, t] = await Promise.all([api(`/api/guilds/${id}`), api(`/api/guilds/${id}/tickets`)])
    guild.value = g
    tickets.value = t.tickets
    if (g.settings) s.value = { ...s.value, ...g.settings, ticket_category_id: g.settings.ticket_category_id === '0' ? null : g.settings.ticket_category_id, mod_role_id: g.settings.mod_role_id === '0' ? null : g.settings.mod_role_id }
    saved.value = JSON.stringify(s.value)
  } catch (e) { error.value = e.message }
})

const backups = ref(null)
const restoring = ref(0)
const exp = ref({ state: 'none' })
let expTimer
async function loadBackups() { try { backups.value = (await api(`/api/guilds/${id}/backups`)).backups } catch { backups.value = [] } }
async function restore(b) {
  if (!confirm(`Restore ${b.what} from ${new Date(b.at * 1000).toLocaleString()}? Your current setup is backed up first.`)) return
  restoring.value = b.id
  try {
    await api(`/api/guilds/${id}/backups/${b.id}/restore`, { method: 'POST' })
    toast.value = { show: true, text: 'Restored. Reloading' }
    setTimeout(() => location.reload(), 800)
  } catch (e) { toast.value = { show: true, text: e.message } }
  restoring.value = 0
}
async function pollExport() {
  try { exp.value = await api(`/api/guilds/${id}/export`) } catch {}
  clearTimeout(expTimer)
  if (exp.value.state === 'working') expTimer = setTimeout(pollExport, 2500)
}
async function startExport() { await api(`/api/guilds/${id}/export`, { method: 'POST' }); exp.value = { state: 'working' }; pollExport() }
watch(tab, t => { if (t === 'settings' && !backups.value) { loadBackups(); pollExport() } }, { immediate: true })
onUnmounted(() => clearTimeout(expTimer))

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
        <v-tab value="panels">Panels</v-tab>
        <v-tab value="agent">AI agent</v-tab>
        <v-tab value="stats">Stats</v-tab>
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
            <div class="section-head" style="align-items:center;margin-bottom:8px">
              <p class="muted small">{{ openCount }} open · {{ tickets.length }} total</p>
              <v-btn-toggle v-model="filter" mandatory density="compact" variant="outlined" divided>
                <v-btn value="open">Open</v-btn><v-btn value="closed">Closed</v-btn><v-btn value="all">All</v-btn>
              </v-btn-toggle>
            </div>
            <p v-if="!shown.length" class="muted small" style="padding:16px 0">Nothing here.</p>
            <router-link v-for="t in shown" :key="t.id" :to="`/t/${t.id}`" class="file-row" style="color:inherit;text-decoration:none">
              <div class="file-main">
                <span class="mono muted">#{{ t.id }}</span>
                <span class="ticket-subject">{{ t.subject || 'No subject' }}</span>
              </div>
              <span v-if="t.priority" :class="['prio', t.priority]">{{ t.priority }}</span>
              <span v-if="t.claimed_by" class="muted small desktop-only">claimed</span>
              <span v-if="t.rating" class="muted small mono">{{ t.rating }}/5</span>
              <span class="muted small mono desktop-only">{{ ago(t.created_at) }}</span>
              <span :class="['status', t.status]">{{ t.status }}</span>
              <v-icon :icon="mdiChevronRight" size="20" class="muted" />
            </router-link>
          </template>
        </v-window-item>
        <v-window-item value="panels">
          <Panels :guild-id="id" :guild="guild" @toast="t => (toast = { show: true, text: t })" />
        </v-window-item>
        <v-window-item value="stats">
          <Stats v-if="tab === 'stats'" :guild-id="id" />
        </v-window-item>
        <v-window-item value="agent">
          <Agent :guild-id="id" @toast="t => (toast = { show: true, text: t })" />
        </v-window-item>
        <v-window-item value="settings">
          <div class="settings-form">
            <v-select v-model="s.ticket_category_id" :items="categories" label="Ticket category" :loading="!guild" hint="New ticket channels go here" persistent-hint class="field-block" />
            <v-select v-model="s.mod_role_id" :items="roles" label="Extra staff role" clearable :loading="!guild" hint="Owner and admins always have access. Optional" persistent-hint class="field-block" />
            <v-select v-model="s.log_channel_id" :items="textChannels" label="Log channel" clearable :loading="!guild" hint="Summaries and transcripts are posted here. Optional" persistent-hint class="field-block" />
            <div class="switch-row">
              <div><h3>AI replies</h3><p class="muted small">Answers members until staff joins. Set it up in AI agent</p></div>
              <v-switch v-model="s.ai_enabled" aria-label="AI replies" />
            </div>
            <v-select v-model="s.autoclose_hours" :items="autoclose" label="Auto-close inactive tickets" hint="Warns the member first, at 75% of the time" persistent-hint class="field-block" />
            <div class="switch-row">
              <div><h3>Close when the member leaves</h3><p class="muted small">Checked every 10 minutes</p></div>
              <v-switch v-model="s.close_on_leave" aria-label="Close when the member leaves" />
            </div>
            <div class="switch-row">
              <div><h3>Ask for a rating</h3><p class="muted small">DMs the member 1 to 5 after closing</p></div>
              <v-switch v-model="s.ratings" aria-label="Ask for a rating" />
            </div>
            <div class="action-row">
              <v-btn color="primary" :loading="saving" :disabled="!dirty || !s.ticket_category_id" @click="save">Save</v-btn>
              <span v-if="dirty" class="muted small">Unsaved changes</span>
            </div>
            <section class="settings-section">
              <h3>Export</h3>
              <p class="muted small">Every ticket, message and file as one zip. The link works for 7 days.</p>
              <div class="action-row">
                <v-btn variant="tonal" :loading="exp.state === 'working'" @click="startExport">Create export</v-btn>
                <v-btn v-if="exp.state === 'done'" variant="text" color="secondary" :href="exp.url">Download zip</v-btn>
                <span v-if="exp.state?.startsWith('failed')" class="error-text small">{{ exp.state }}</span>
              </div>
            </section>
            <section class="settings-section">
              <h3>Backups</h3>
              <p class="muted small">Saved before every change to settings, panels or the AI agent. Last 50 kept.</p>
              <p v-if="backups && !backups.length" class="muted small">No backups yet.</p>
              <div v-for="b in backups || []" :key="b.id" class="file-row">
                <div class="file-main"><span>{{ new Date(b.at * 1000).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' }) }}</span><span class="muted small">before {{ b.what }} change</span></div>
                <v-btn variant="text" size="small" :loading="restoring === b.id" @click="restore(b)">Restore</v-btn>
              </div>
            </section>
          </div>
        </v-window-item>
      </v-window>
    </template>
    <v-snackbar v-model="toast.show" :timeout="2500">{{ toast.text }}</v-snackbar>
  </div>
</template>
