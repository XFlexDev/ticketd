<script setup>
import { computed, onMounted, ref } from 'vue'
import { api } from '../api'

const props = defineProps({ guildId: String, guild: Object })
const emit = defineEmits(['toast'])
const cfg = ref({ role_id: null, remove_role_id: null, title: '', description: '', color: '#66BB6A', button: '' })
const saved = ref('')
const saving = ref(false)
const sending = ref(false)
const sendTo = ref(null)
const error = ref('')
const roles = computed(() => (props.guild?.roles || []).map(r => ({ title: r.name, value: r.id })))
const textChannels = computed(() => (props.guild?.channels || []).filter(c => c.type === 0 || c.type === 5).map(c => ({ title: `#${c.name}`, value: c.id })))
const body = () => ({ ...cfg.value, role_id: cfg.value.role_id || '', remove_role_id: cfg.value.remove_role_id || '' })
const dirty = computed(() => JSON.stringify(body()) !== saved.value)

onMounted(async () => {
  try {
    const r = await api(`/api/guilds/${props.guildId}/verify`)
    cfg.value = { ...cfg.value, ...r.config, role_id: r.config.role_id || null, remove_role_id: r.config.remove_role_id || null, color: r.config.color || '#66BB6A' }
    saved.value = r.config.role_id ? JSON.stringify(body()) : ''
  } catch (e) { error.value = e.message }
})

async function save() {
  saving.value = true
  error.value = ''
  try {
    await api(`/api/guilds/${props.guildId}/verify`, { method: 'PUT', body: JSON.stringify(body()) })
    saved.value = JSON.stringify(body())
    emit('toast', 'Verification saved')
  } catch (e) { error.value = e.message }
  saving.value = false
}

async function send() {
  sending.value = true
  error.value = ''
  try {
    if (dirty.value) await save()
    await api(`/api/guilds/${props.guildId}/verify/send`, { method: 'POST', body: JSON.stringify({ channel_id: sendTo.value }) })
    emit('toast', 'Verify message posted')
  } catch (e) { error.value = e.message }
  sending.value = false
}
</script>

<template>
  <div class="settings-form">
    <p class="muted small" style="margin:0 0 20px">New members press Verify, solve a free captcha on kev.kreatix.dev and get a role. Lock your channels to that role so bots and raiders never get in.</p>
    <v-select v-model="cfg.role_id" :items="roles" label="Role to give" :loading="!guild" hint="Kev's role must be above it in Server Settings, Roles" persistent-hint class="field-block" />
    <v-select v-model="cfg.remove_role_id" :items="roles" label="Role to take away" clearable :loading="!guild" hint="Optional, for example Unverified" persistent-hint class="field-block" />
    <v-text-field v-model="cfg.title" label="Title" placeholder="Verify you're human" persistent-placeholder class="field-block" />
    <v-textarea v-model="cfg.description" label="Message" placeholder="Press the button below and solve a quick captcha to get access to the server." persistent-placeholder rows="3" auto-grow class="field-block" />
    <div class="field-block" style="display:flex;gap:16px">
      <v-text-field v-model="cfg.button" label="Button text" placeholder="Verify" persistent-placeholder style="flex:2" />
      <v-text-field v-model="cfg.color" label="Color" style="flex:1" />
    </div>
    <div class="action-row">
      <v-btn color="primary" :loading="saving" :disabled="!dirty || !cfg.role_id" @click="save">Save</v-btn>
      <span v-if="dirty" class="muted small">Unsaved changes</span>
    </div>
    <section class="settings-section">
      <h3>Post the Verify message</h3>
      <div class="send-row">
        <v-select v-model="sendTo" :items="textChannels" label="Channel" hide-details density="comfortable" />
        <v-btn color="primary" :loading="sending" :disabled="!sendTo || !cfg.role_id" @click="send">Post</v-btn>
      </div>
    </section>
    <p v-if="error" class="error-text small">{{ error }}</p>
  </div>
</template>
