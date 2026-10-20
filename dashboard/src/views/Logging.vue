<script setup>
import { computed, onMounted, ref } from 'vue'
import { api } from '../api'

const props = defineProps({ guildId: String, guild: Object })
const emit = defineEmits(['toast'])
const cfg = ref(null)
const saved = ref('')
const saving = ref(false)
const error = ref('')
const textChannels = computed(() => (props.guild?.channels || []).filter(c => c.type === 0 || c.type === 5).map(c => ({ title: `#${c.name}`, value: c.id })))
const kinds = [
  { key: 'messages', title: 'Messages', hint: 'Edits, deletes and purges' },
  { key: 'members', title: 'Members', hint: 'Joins, leaves, nicknames, roles, bans' },
  { key: 'server', title: 'Server', hint: 'Channels and roles created or deleted' },
  { key: 'voice', title: 'Voice', hint: 'Joins, leaves and moves' },
]
const body = () => Object.fromEntries(Object.entries(cfg.value).map(([k, v]) => [k, v ?? '']))
const dirty = computed(() => cfg.value && JSON.stringify(body()) !== saved.value)
function load(c) {
  cfg.value = { ...c, ...Object.fromEntries(kinds.map(k => [k.key, c[k.key] || null])) }
  saved.value = JSON.stringify(body())
}
onMounted(async () => {
  try { load((await api(`/api/guilds/${props.guildId}/modules/logging`)).config) } catch (e) { error.value = e.message }
})
async function save() {
  saving.value = true
  error.value = ''
  try {
    load((await api(`/api/guilds/${props.guildId}/modules/logging`, { method: 'PUT', body: JSON.stringify(body()) })).config)
    emit('toast', 'Logging saved')
  } catch (e) { error.value = e.message }
  saving.value = false
}
</script>

<template>
  <div class="settings-form">
    <div v-if="!cfg && !error" class="row-list"><div v-for="i in 3" :key="i" class="skeleton-row"><span /></div></div>
    <template v-if="cfg">
      <p class="muted small" style="margin:0 0 20px">Pick a channel for each kind of log, or leave it empty to turn it off. The same channel works for all of them.</p>
      <v-select v-for="k in kinds" :key="k.key" v-model="cfg[k.key]" :items="textChannels" :label="k.title" :hint="k.hint" persistent-hint clearable :loading="!guild" class="field-block" />
      <v-select v-model="cfg.ignored_channels" :items="textChannels" label="Ignored channels" multiple chips closable-chips :loading="!guild" class="field-block" />
      <div class="action-row">
        <v-btn color="primary" :loading="saving" :disabled="!dirty" @click="save">Save</v-btn>
        <span v-if="dirty" class="muted small">Unsaved changes</span>
      </div>
    </template>
    <p v-if="error" class="error-text small">{{ error }}</p>
  </div>
</template>
