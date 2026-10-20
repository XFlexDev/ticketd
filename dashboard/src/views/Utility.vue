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
const body = () => ({ ...cfg.value, starboard_channel_id: cfg.value.starboard_channel_id || '', suggestions_channel_id: cfg.value.suggestions_channel_id || '' })
const dirty = computed(() => cfg.value && JSON.stringify(body()) !== saved.value)

function load(c) {
  cfg.value = { ...c, starboard_channel_id: c.starboard_channel_id || null, suggestions_channel_id: c.suggestions_channel_id || null }
  saved.value = JSON.stringify(body())
}
onMounted(async () => {
  try { load((await api(`/api/guilds/${props.guildId}/modules/utility`)).config) } catch (e) { error.value = e.message }
})
async function save() {
  saving.value = true
  error.value = ''
  try {
    load((await api(`/api/guilds/${props.guildId}/modules/utility`, { method: 'PUT', body: JSON.stringify(body()) })).config)
    emit('toast', 'Utility saved')
  } catch (e) { error.value = e.message }
  saving.value = false
}
</script>

<template>
  <div class="settings-form">
    <div v-if="!cfg && !error" class="row-list"><div v-for="i in 3" :key="i" class="skeleton-row"><span /></div></div>
    <template v-if="cfg">
      <div class="setting-row">
        <div><b>Starboard</b><p class="muted small">Messages that get enough reactions are copied to a channel.</p></div>
        <v-switch v-model="cfg.starboard_on" color="primary" hide-details aria-label="Starboard" />
      </div>
      <template v-if="cfg.starboard_on">
        <v-select v-model="cfg.starboard_channel_id" :items="textChannels" label="Starboard channel" :loading="!guild" hide-details density="comfortable" class="field-block" />
        <div class="item-row">
          <v-text-field v-model="cfg.star_emoji" label="Emoji" hint="Any emoji, or a server emoji like <:name:id>" persistent-hint density="comfortable" />
          <v-text-field v-model.number="cfg.star_threshold" type="number" min="1" max="100" label="Reactions needed" density="comfortable" hide-details />
        </div>
      </template>

      <h3 class="section-title">Suggestions</h3>
      <p class="muted small" style="margin:0 0 12px">Members post ideas with <code>/suggest</code>. Everyone can vote, staff approve or deny.</p>
      <v-select v-model="cfg.suggestions_channel_id" :items="textChannels" label="Suggestions channel" clearable :loading="!guild" hide-details density="comfortable" class="field-block" />

      <h3 class="section-title">Commands</h3>
      <p class="muted small" style="margin:0 0 12px"><code>/userinfo</code> <code>/serverinfo</code> <code>/avatar</code> <code>/banner</code> <code>/roleinfo</code> <code>/afk</code>, and <code>/sticky set</code> keeps a message at the bottom of a channel.</p>

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
.setting-row { display: flex; justify-content: space-between; align-items: center; gap: 16px; margin-bottom: 12px }
.setting-row p { margin: 4px 0 0 }
.item-row { display: flex; gap: 10px; margin: 0 0 16px; flex-wrap: wrap; align-items: center }
.item-row > * { flex: 1; min-width: 160px }
</style>
