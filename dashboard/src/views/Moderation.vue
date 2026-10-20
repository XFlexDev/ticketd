<script setup>
import { computed, onMounted, ref } from 'vue'
import { api } from '../api'

const props = defineProps({ guildId: String, guild: Object })
const emit = defineEmits(['toast'])
const cfg = ref(null)
const saved = ref('')
const saving = ref(false)
const error = ref('')
const cases = ref(null)
const textChannels = computed(() => (props.guild?.channels || []).filter(c => c.type === 0 || c.type === 5).map(c => ({ title: `#${c.name}`, value: c.id })))
const actions = [{ title: 'Timeout', value: 'timeout' }, { title: 'Kick', value: 'kick' }, { title: 'Ban', value: 'ban' }]
const body = () => ({ ...cfg.value, log_channel_id: cfg.value.log_channel_id || '' })
const dirty = computed(() => cfg.value && JSON.stringify(body()) !== saved.value)

onMounted(async () => {
  try {
    const [m, c] = await Promise.all([api(`/api/guilds/${props.guildId}/modules/moderation`), api(`/api/guilds/${props.guildId}/cases`)])
    cfg.value = { ...m.config, log_channel_id: m.config.log_channel_id || null }
    saved.value = JSON.stringify(body())
    cases.value = c.cases
  } catch (e) { error.value = e.message }
})

function addStep() {
  const last = cfg.value.escalation.at(-1)
  cfg.value.escalation.push({ warns: (last?.warns || 0) + 2, action: 'timeout', minutes: 60 })
}

async function save() {
  saving.value = true
  error.value = ''
  try {
    const r = await api(`/api/guilds/${props.guildId}/modules/moderation`, { method: 'PUT', body: JSON.stringify(body()) })
    cfg.value = { ...r.config, log_channel_id: r.config.log_channel_id || null }
    saved.value = JSON.stringify(body())
    emit('toast', 'Moderation saved')
  } catch (e) { error.value = e.message }
  saving.value = false
}

function when(sec) { return new Date(sec * 1000).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' }) }
function dur(s) {
  if (!s) return ''
  for (const [n, u] of [[604800, 'w'], [86400, 'd'], [3600, 'h'], [60, 'm']]) if (s % n === 0) return `${s / n}${u}`
  return `${s}s`
}
</script>

<template>
  <div class="settings-form">
    <p class="muted small" style="margin:0 0 20px">Use <code>/ban</code> <code>/kick</code> <code>/timeout</code> <code>/warn</code> <code>/purge</code> <code>/lock</code> <code>/slowmode</code> in Discord. Every action gets a case number and lands in the mod log.</p>
    <template v-if="cfg">
      <v-select v-model="cfg.log_channel_id" :items="textChannels" label="Mod log channel" clearable :loading="!guild" hint="Every case is posted here, automod too" persistent-hint class="field-block" />
      <div class="switch-row">
        <div><h3>DM the member</h3><p class="muted small">Tells them what happened and why, before the action</p></div>
        <v-switch v-model="cfg.dm" aria-label="DM the member" />
      </div>
      <section class="settings-section">
        <h3>Automatic punishments</h3>
        <p class="muted small">When someone reaches a number of active warnings.</p>
        <div v-for="(st, i) in cfg.escalation" :key="i" class="step-row">
          <v-text-field v-model.number="st.warns" type="number" min="1" label="Warnings" hide-details density="comfortable" style="max-width:110px" />
          <v-select v-model="st.action" :items="actions" label="Then" hide-details density="comfortable" style="max-width:150px" />
          <v-text-field v-if="st.action !== 'kick'" v-model.number="st.minutes" type="number" min="0" :label="st.action === 'ban' ? 'Minutes (0 = forever)' : 'Minutes'" hide-details density="comfortable" style="max-width:170px" />
          <v-btn variant="text" size="small" @click="cfg.escalation.splice(i, 1)">Remove</v-btn>
        </div>
        <v-btn variant="tonal" size="small" :disabled="cfg.escalation.length >= 10" @click="addStep">Add step</v-btn>
      </section>
      <div class="action-row">
        <v-btn color="primary" :loading="saving" :disabled="!dirty" @click="save">Save</v-btn>
        <span v-if="dirty" class="muted small">Unsaved changes</span>
      </div>
    </template>
    <div v-else-if="!error" class="row-list"><div v-for="i in 3" :key="i" class="skeleton-row"><span /></div></div>
    <p v-if="error" class="error-text small">{{ error }}</p>
    <section class="settings-section">
      <h3>Recent cases</h3>
      <p v-if="cases && !cases.length" class="muted small">No cases yet.</p>
      <div v-for="c in cases || []" :key="c.num" class="file-row">
        <div class="file-main">
          <span><span class="mono muted">#{{ c.num }}</span> <span :class="['case-act', c.action]">{{ c.action }}</span> {{ c.user_name }}<span v-if="c.duration" class="muted"> · {{ dur(c.duration) }}</span></span>
          <span class="muted small" :style="c.action === 'warn' && !c.active ? 'text-decoration:line-through' : ''">{{ c.reason }} · by {{ c.mod_name }}</span>
        </div>
        <span class="muted small mono desktop-only">{{ when(c.created_at) }}</span>
      </div>
    </section>
  </div>
</template>

<style scoped>
.step-row { display: flex; gap: 10px; align-items: center; flex-wrap: wrap; margin: 10px 0 }
.case-act { font-weight: 600; text-transform: capitalize }
.case-act.ban, .case-act.kick { color: #ef5350 }
.case-act.timeout, .case-act.warn { color: #ffb74d }
.case-act.unban, .case-act.untimeout { color: #66bb6a }
</style>
