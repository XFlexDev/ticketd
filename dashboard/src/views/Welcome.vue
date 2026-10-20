<script setup>
import { computed, onMounted, ref } from 'vue'
import { api } from '../api'

const props = defineProps({ guildId: String, guild: Object })
const emit = defineEmits(['toast'])
const cfg = ref(null)
const saved = ref('')
const saving = ref(false)
const error = ref('')
const roles = computed(() => (props.guild?.roles || []).map(r => ({ title: r.name, value: r.id })))
const textChannels = computed(() => (props.guild?.channels || []).filter(c => c.type === 0 || c.type === 5).map(c => ({ title: `#${c.name}`, value: c.id })))
const body = () => ({ ...cfg.value, welcome_channel_id: cfg.value.welcome_channel_id || '', leave_channel_id: cfg.value.leave_channel_id || '' })
const dirty = computed(() => cfg.value && JSON.stringify(body()) !== saved.value)
const preview = computed(() => (cfg.value?.welcome_message || '').replaceAll('{mention}', '@you').replaceAll('{user}', 'you').replaceAll('{username}', 'you').replaceAll('{server}', props.guild?.name || 'Server').replaceAll('{count}', '128'))

function load(c) {
  cfg.value = { ...c, welcome_channel_id: c.welcome_channel_id || null, leave_channel_id: c.leave_channel_id || null }
  saved.value = JSON.stringify(body())
}
onMounted(async () => {
  try { load((await api(`/api/guilds/${props.guildId}/modules/welcome`)).config) } catch (e) { error.value = e.message }
})
async function save() {
  saving.value = true
  error.value = ''
  try {
    load((await api(`/api/guilds/${props.guildId}/modules/welcome`, { method: 'PUT', body: JSON.stringify(body()) })).config)
    emit('toast', 'Welcome saved')
  } catch (e) { error.value = e.message }
  saving.value = false
}
</script>

<template>
  <div class="settings-form">
    <div v-if="!cfg && !error" class="row-list"><div v-for="i in 3" :key="i" class="skeleton-row"><span /></div></div>
    <template v-if="cfg">
      <p class="muted small" style="margin:0 0 20px">Use <code>{mention}</code> <code>{user}</code> <code>{server}</code> <code>{count}</code> in messages.</p>
      <div class="switch-row">
        <div><h3>Welcome message</h3><p class="muted small">Posted when someone joins</p></div>
        <v-switch v-model="cfg.welcome_on" aria-label="Welcome message" />
      </div>
      <template v-if="cfg.welcome_on">
        <v-select v-model="cfg.welcome_channel_id" :items="textChannels" label="Channel" :loading="!guild" class="field-block" />
        <v-textarea v-model="cfg.welcome_message" label="Message" rows="2" auto-grow class="field-block" />
        <div class="switch-row">
          <div><h3>As an embed</h3><p class="muted small">With their avatar, a title and an optional image</p></div>
          <v-switch v-model="cfg.embed" aria-label="As an embed" />
        </div>
        <div v-if="cfg.embed" class="field-block" style="display:flex;gap:16px">
          <v-text-field v-model="cfg.title" label="Title" style="flex:2" />
          <v-text-field v-model="cfg.color" label="Color" style="flex:1" />
        </div>
        <v-text-field v-if="cfg.embed" v-model="cfg.image" label="Image link" placeholder="https://" persistent-placeholder class="field-block" />
        <div class="wl-preview" :style="{ borderColor: cfg.embed ? cfg.color : 'transparent' }">
          <b v-if="cfg.embed && cfg.title">{{ cfg.title }}</b>
          <p>{{ preview }}</p>
          <img v-if="cfg.embed && cfg.image" :src="cfg.image" alt="">
        </div>
      </template>
      <div class="switch-row">
        <div><h3>Welcome DM</h3><p class="muted small">A private message to the new member</p></div>
        <v-switch v-model="cfg.dm_on" aria-label="Welcome DM" />
      </div>
      <v-textarea v-if="cfg.dm_on" v-model="cfg.dm_message" label="DM" rows="2" auto-grow class="field-block" />
      <div class="switch-row">
        <div><h3>Leave message</h3><p class="muted small">Posted when someone leaves</p></div>
        <v-switch v-model="cfg.leave_on" aria-label="Leave message" />
      </div>
      <template v-if="cfg.leave_on">
        <v-select v-model="cfg.leave_channel_id" :items="textChannels" label="Channel" :loading="!guild" class="field-block" />
        <v-textarea v-model="cfg.leave_message" label="Message" rows="2" auto-grow class="field-block" />
      </template>
      <section class="settings-section">
        <h3>Auto roles</h3>
        <p class="muted small">Given on join. Kev's role must be above them.</p>
        <v-select v-model="cfg.autoroles" :items="roles" label="Roles for members" multiple chips closable-chips :loading="!guild" class="field-block" />
        <v-select v-model="cfg.bot_roles" :items="roles" label="Roles for bots" multiple chips closable-chips :loading="!guild" class="field-block" />
      </section>
      <div class="action-row">
        <v-btn color="primary" :loading="saving" :disabled="!dirty" @click="save">Save</v-btn>
        <span v-if="dirty" class="muted small">Unsaved changes</span>
      </div>
    </template>
    <p v-if="error" class="error-text small">{{ error }}</p>
  </div>
</template>

<style scoped>
.wl-preview { background: #111214; border-left: 4px solid; border-radius: 6px; padding: 12px 14px; margin: 0 0 20px }
.wl-preview p { margin: 4px 0 0; white-space: pre-wrap }
.wl-preview img { max-width: 100%; border-radius: 6px; margin-top: 10px }
</style>
