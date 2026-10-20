<script setup>
import { computed, onMounted, ref } from 'vue'
import { api } from '../api'

const props = defineProps({ guildId: String, guild: Object })
const emit = defineEmits(['toast'])
const cfg = ref(null)
const saved = ref('')
const saving = ref(false)
const error = ref('')
const confirmReset = ref(false)
const roles = computed(() => (props.guild?.roles || []).map(r => ({ title: r.name, value: r.id })))
const textChannels = computed(() => (props.guild?.channels || []).filter(c => c.type === 0 || c.type === 5).map(c => ({ title: `#${c.name}`, value: c.id })))
const announce = [{ title: 'In the same channel', value: 'current' }, { title: 'In one channel', value: 'channel' }, { title: 'By DM', value: 'dm' }, { title: 'Off', value: 'off' }]
const body = () => ({ ...cfg.value, announce_channel_id: cfg.value.announce_channel_id || '' })
const dirty = computed(() => cfg.value && JSON.stringify(body()) !== saved.value)
const link = computed(() => `${location.origin}/leaderboard/${props.guildId}`)

function load(c) {
  cfg.value = { ...c, announce_channel_id: c.announce_channel_id || null }
  saved.value = JSON.stringify(body())
}
onMounted(async () => {
  try { load((await api(`/api/guilds/${props.guildId}/modules/leveling`)).config) } catch (e) { error.value = e.message }
})
async function save() {
  saving.value = true
  error.value = ''
  try {
    load((await api(`/api/guilds/${props.guildId}/modules/leveling`, { method: 'PUT', body: JSON.stringify(body()) })).config)
    emit('toast', 'Leveling saved')
  } catch (e) { error.value = e.message }
  saving.value = false
}
async function reset() {
  if (!confirmReset.value) { confirmReset.value = true; return }
  try {
    await api(`/api/guilds/${props.guildId}/xp/reset`, { method: 'POST' })
    emit('toast', 'All XP reset')
  } catch (e) { error.value = e.message }
  confirmReset.value = false
}
function addReward() {
  const last = cfg.value.rewards.at(-1)
  cfg.value.rewards.push({ level: (last?.level || 0) + 5, role_id: null })
}
</script>

<template>
  <div class="settings-form">
    <div v-if="!cfg && !error" class="row-list"><div v-for="i in 3" :key="i" class="skeleton-row"><span /></div></div>
    <template v-if="cfg">
      <div class="switch-row">
        <div><h3>Leveling</h3><p class="muted small">Members earn XP by chatting. <code>/rank</code> <code>/leaderboard</code>, and <code>/xp</code> for admins</p></div>
        <v-switch v-model="cfg.enabled" aria-label="Leveling" />
      </div>
      <div :class="{ off: !cfg.enabled }">
        <div class="lv-row">
          <v-text-field v-model.number="cfg.min_xp" type="number" label="Min XP per message" hide-details density="comfortable" />
          <v-text-field v-model.number="cfg.max_xp" type="number" label="Max XP per message" hide-details density="comfortable" />
          <v-text-field v-model.number="cfg.cooldown" type="number" label="Cooldown (seconds)" hide-details density="comfortable" />
        </div>
        <p class="muted small" style="margin:6px 0 20px">One message per cooldown counts, so spamming doesn't help.</p>
        <v-select v-model="cfg.announce" :items="announce" label="Level up message" class="field-block" />
        <v-select v-if="cfg.announce === 'channel'" v-model="cfg.announce_channel_id" :items="textChannels" label="Channel" :loading="!guild" class="field-block" />
        <v-textarea v-if="cfg.announce !== 'off'" v-model="cfg.message" label="Message" hint="{mention} {user} {level}" persistent-hint rows="1" auto-grow class="field-block" />

        <section class="settings-section">
          <h3>Reward roles</h3>
          <p class="muted small">Given when a member reaches the level. Kev's role must be above them.</p>
          <div v-for="(r, i) in cfg.rewards" :key="i" class="lv-row">
            <v-text-field v-model.number="r.level" type="number" min="1" label="Level" hide-details density="comfortable" style="max-width:100px" />
            <v-select v-model="r.role_id" :items="roles" label="Role" hide-details density="comfortable" :loading="!guild" />
            <v-btn variant="text" size="small" @click="cfg.rewards.splice(i, 1)">Remove</v-btn>
          </div>
          <v-btn variant="tonal" size="small" @click="addReward">Add reward</v-btn>
          <div class="switch-row">
            <div><h3>Keep old rewards</h3><p class="muted small">Off means members only keep their highest reward role</p></div>
            <v-switch v-model="cfg.stack" aria-label="Keep old rewards" />
          </div>
        </section>
        <v-select v-model="cfg.no_xp_channels" :items="textChannels" label="No XP in these channels" multiple chips closable-chips :loading="!guild" class="field-block" style="margin-top:20px" />
        <v-select v-model="cfg.no_xp_roles" :items="roles" label="No XP for these roles" multiple chips closable-chips :loading="!guild" class="field-block" />
      </div>
      <div class="action-row">
        <v-btn color="primary" :loading="saving" :disabled="!dirty" @click="save">Save</v-btn>
        <v-btn variant="text" :href="link" target="_blank" :disabled="!cfg.enabled">Public leaderboard</v-btn>
        <v-btn variant="text" :color="confirmReset ? 'error' : undefined" @click="reset">{{ confirmReset ? 'Tap again to reset all XP' : 'Reset all XP' }}</v-btn>
      </div>
    </template>
    <p v-if="error" class="error-text small">{{ error }}</p>
  </div>
</template>

<style scoped>
.lv-row { display: flex; gap: 10px; align-items: center; margin: 0 0 10px; flex-wrap: wrap }
.lv-row > * { flex: 1; min-width: 140px }
.off { opacity: .45; transition: opacity .25s ease }
</style>
