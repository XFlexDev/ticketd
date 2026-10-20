<script setup>
import { computed, onMounted, ref } from 'vue'
import { api } from '../api'

const props = defineProps({ guildId: String, guild: Object })
const emit = defineEmits(['toast'])
const cfg = ref(null)
const saved = ref('')
const saving = ref(false)
const making = ref(false)
const error = ref('')
const extra = ref([])
const roles = computed(() => (props.guild?.roles || []).map(r => ({ title: r.name, value: r.id })))
const voices = computed(() => [...(props.guild?.channels || []), ...extra.value].filter(c => c.type === 2).map(c => ({ title: c.name, value: c.id })))
const categories = computed(() => (props.guild?.channels || []).filter(c => c.type === 4).map(c => ({ title: c.name, value: c.id })))
const category = ref(null)
const body = () => ({ ...cfg.value, hub_channel_id: cfg.value.hub_channel_id || '', dj_role_id: cfg.value.dj_role_id || '' })
const dirty = computed(() => cfg.value && JSON.stringify(body()) !== saved.value)

function load(c) {
  cfg.value = { ...c, hub_channel_id: c.hub_channel_id || null, dj_role_id: c.dj_role_id || null }
  saved.value = JSON.stringify(body())
}
onMounted(async () => {
  try { load((await api(`/api/guilds/${props.guildId}/modules/music`)).config) } catch (e) { error.value = e.message }
})
async function save() {
  saving.value = true
  error.value = ''
  try {
    load((await api(`/api/guilds/${props.guildId}/modules/music`, { method: 'PUT', body: JSON.stringify(body()) })).config)
    emit('toast', 'Music saved')
  } catch (e) { error.value = e.message }
  saving.value = false
}
async function makeHub() {
  making.value = true
  error.value = ''
  try {
    const r = await api(`/api/guilds/${props.guildId}/music/hub`, { method: 'POST', body: JSON.stringify({ category_id: category.value || '' }) })
    extra.value.push(r.channel)
    load(r.config)
    emit('toast', 'Made "Join to create"')
  } catch (e) { error.value = e.message }
  making.value = false
}
</script>

<template>
  <div class="settings-form">
    <div v-if="!cfg && !error" class="row-list"><div v-for="i in 3" :key="i" class="skeleton-row"><span /></div></div>
    <template v-if="cfg">
      <div class="setting-row">
        <div><b>Music</b><p class="muted small">Members play songs with <code>/play</code>, stations with <code>/radio</code>, or upload audio files. Everyone in the channel shares the queue.</p></div>
        <v-switch v-model="cfg.enabled" color="primary" hide-details aria-label="Music" />
      </div>

      <h3 class="section-title">Lobbies</h3>
      <p class="muted small" style="margin:0 0 12px">Joining the hub channel makes a private lobby for that member and moves them in. The owner controls the music there. Empty lobbies get deleted.</p>
      <v-select v-model="cfg.hub_channel_id" :items="voices" label="Hub channel" clearable :loading="!guild" hide-details density="comfortable" class="field-block" />
      <div class="item-row">
        <v-select v-model="category" :items="categories" label="Category for a new hub" clearable hide-details density="comfortable" />
        <v-btn variant="tonal" :loading="making" @click="makeHub">Make a hub channel</v-btn>
      </div>
      <div class="item-row">
        <v-text-field v-model="cfg.lobby_name" label="Lobby name" hint="{user} is the member's name" persistent-hint density="comfortable" />
        <v-text-field v-model.number="cfg.user_limit" type="number" min="0" max="99" label="Member limit" hint="0 means no limit" persistent-hint density="comfortable" style="max-width:160px" />
      </div>

      <h3 class="section-title">Control</h3>
      <v-select v-model="cfg.dj_role_id" :items="roles" label="DJ role" hint="Can skip, pause, stop and change volume anywhere. Admins and lobby owners always can" persistent-hint clearable :loading="!guild" density="comfortable" class="field-block" />
      <div class="item-row">
        <v-text-field v-model.number="cfg.vote_percent" type="number" min="1" max="100" label="Votes to skip" suffix="% of listeners" density="comfortable" hide-details />
        <v-text-field v-model.number="cfg.max_queue" type="number" min="1" max="500" label="Queue limit" suffix="songs" density="comfortable" hide-details />
      </div>

      <div class="action-row">
        <v-btn color="primary" :loading="saving" :disabled="!dirty" @click="save">Save</v-btn>
        <span v-if="dirty" class="muted small">Unsaved changes</span>
      </div>
    </template>
    <p v-if="error" class="error-text small">{{ error }}</p>
  </div>
</template>

<style scoped>
.section-title { font-size: 15px; font-weight: 600; margin: 28px 0 6px }
.setting-row { display: flex; justify-content: space-between; align-items: center; gap: 16px }
.setting-row p { margin: 4px 0 0 }
.item-row { display: flex; gap: 10px; margin: 0 0 16px; flex-wrap: wrap; align-items: center }
.item-row > * { flex: 1; min-width: 160px }
</style>
