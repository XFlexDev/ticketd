<script setup>
import { computed, onMounted, ref } from 'vue'
import { api } from '../api'
import ImageField from '../components/ImageField.vue'

const props = defineProps({ guildId: String, guild: Object })
const emit = defineEmits(['toast'])
const cfg = ref(null)
const saved = ref('')
const saving = ref(false)
const error = ref('')
const roles = computed(() => (props.guild?.roles || []).map(r => ({ title: r.name, value: r.id })))
const textChannels = computed(() => (props.guild?.channels || []).filter(c => c.type === 0 || c.type === 5).map(c => ({ title: `#${c.name}`, value: c.id })))
const modes = [{ title: 'Contains', value: 'contains' }, { title: 'Starts with', value: 'starts' }, { title: 'Is exactly', value: 'exact' }]
const body = () => ({ commands: cfg.value.commands.map(c => ({ ...c, role_id: c.role_id || '' })), responders: cfg.value.responders })
const dirty = computed(() => cfg.value && JSON.stringify(body()) !== saved.value)
const fill = s => (s || '').replaceAll('{mention}', '@you').replaceAll('{user}', 'you').replaceAll('{username}', 'you').replaceAll('{server}', props.guild?.name || 'Server').replaceAll('{channel}', '#general')
const embed = ref({ channel_id: null, content: '', title: '', description: '', url: '', color: '#66BB6A', image: '', thumbnail: '', author: '', footer: '' })
const sending = ref(false)

function load(c) {
  cfg.value = { commands: c.commands.map(x => ({ ...x, role_id: x.role_id || null })), responders: c.responders }
  saved.value = JSON.stringify(body())
}
onMounted(async () => {
  try { load((await api(`/api/guilds/${props.guildId}/modules/commands`)).config) } catch (e) { error.value = e.message }
})
const blank = { text: '', embed: false, title: '', color: '#66BB6A', image: '' }
function addCommand() { cfg.value.commands.push({ id: '', name: '', description: '', private: false, role_id: null, ...blank }) }
function addResponder() { cfg.value.responders.push({ id: '', trigger: '', mode: 'contains', reply: true, ...blank }) }
async function save() {
  saving.value = true
  error.value = ''
  try {
    load((await api(`/api/guilds/${props.guildId}/modules/commands`, { method: 'PUT', body: JSON.stringify(body()) })).config)
    emit('toast', 'Saved. New commands show up in Discord in a few seconds')
  } catch (e) { error.value = e.message }
  saving.value = false
}
async function sendEmbed() {
  sending.value = true
  error.value = ''
  try {
    await api(`/api/guilds/${props.guildId}/embed`, { method: 'POST', body: JSON.stringify(embed.value) })
    emit('toast', 'Posted')
  } catch (e) { error.value = e.message }
  sending.value = false
}
</script>

<template>
  <div class="settings-form">
    <div v-if="!cfg && !error" class="row-list"><div v-for="i in 3" :key="i" class="skeleton-row"><span /></div></div>
    <template v-if="cfg">
      <h3 class="section-title">Slash commands</h3>
      <p class="muted small" style="margin:0 0 12px">Your own commands, like <code>/rules</code> or <code>/ip</code>. Responses can use <code>{mention}</code> <code>{user}</code> <code>{server}</code> <code>{channel}</code>.</p>
      <div v-for="(c, i) in cfg.commands" :key="'c' + i" class="item">
        <div class="item-row">
          <v-text-field v-model="c.name" label="Name" prefix="/" hide-details density="comfortable" />
          <v-text-field v-model="c.description" label="Description" placeholder="Custom command" persistent-placeholder hide-details density="comfortable" />
        </div>
        <v-textarea v-model="c.text" label="Response" rows="2" auto-grow hide-details class="field-block" />
        <div class="item-row">
          <v-select v-model="c.role_id" :items="roles" label="Only for role" clearable hide-details density="comfortable" />
          <v-switch v-model="c.private" label="Only the user sees it" hide-details color="primary" />
          <v-switch v-model="c.embed" label="Embed" hide-details color="primary" />
        </div>
        <template v-if="c.embed">
          <div class="item-row">
            <v-text-field v-model="c.title" label="Embed title" hide-details density="comfortable" />
            <v-text-field v-model="c.color" label="Color" type="color" hide-details density="comfortable" style="max-width:120px" />
          </div>
          <ImageField v-model="c.image" label="Image" :guild-id="guildId" class="field-block" @error="e => (error = e)" />
        </template>
        <div class="preview" :style="{ borderColor: c.embed ? c.color : 'transparent' }">
          <b v-if="c.embed && c.title">{{ fill(c.title) }}</b>
          <p>{{ fill(c.text) || ' ' }}</p>
          <img v-if="c.embed && c.image" :src="c.image" alt="">
        </div>
        <v-btn variant="text" size="small" @click="cfg.commands.splice(i, 1)">Remove /{{ c.name || 'command' }}</v-btn>
      </div>
      <div class="action-row"><v-btn variant="tonal" size="small" :disabled="cfg.commands.length >= 50" @click="addCommand">Add command</v-btn></div>

      <h3 class="section-title">Auto replies</h3>
      <p class="muted small" style="margin:0 0 12px">Kev answers when a message matches. Same reply in a channel at most every 5 seconds.</p>
      <div v-for="(r, i) in cfg.responders" :key="'r' + i" class="item">
        <div class="item-row">
          <v-select v-model="r.mode" :items="modes" label="When a message" hide-details density="comfortable" style="max-width:180px" />
          <v-text-field v-model="r.trigger" label="Trigger" placeholder="how do i join" persistent-placeholder hide-details density="comfortable" />
        </div>
        <v-textarea v-model="r.text" label="Reply" rows="2" auto-grow hide-details class="field-block" />
        <div class="item-row">
          <v-switch v-model="r.reply" label="Reply to the message" hide-details color="primary" />
          <v-switch v-model="r.embed" label="Embed" hide-details color="primary" />
        </div>
        <template v-if="r.embed">
          <div class="item-row">
            <v-text-field v-model="r.title" label="Embed title" hide-details density="comfortable" />
            <v-text-field v-model="r.color" label="Color" type="color" hide-details density="comfortable" style="max-width:120px" />
          </div>
          <ImageField v-model="r.image" label="Image" :guild-id="guildId" class="field-block" @error="e => (error = e)" />
        </template>
        <v-btn variant="text" size="small" @click="cfg.responders.splice(i, 1)">Remove</v-btn>
      </div>
      <div class="action-row"><v-btn variant="tonal" size="small" :disabled="cfg.responders.length >= 50" @click="addResponder">Add auto reply</v-btn></div>

      <div class="action-row">
        <v-btn color="primary" :loading="saving" :disabled="!dirty" @click="save">Save</v-btn>
        <span v-if="dirty" class="muted small">Unsaved changes</span>
      </div>

      <h3 class="section-title">Send an embed</h3>
      <v-textarea v-model="embed.content" label="Message above the embed" rows="1" auto-grow hide-details class="field-block" />
      <div class="item-row">
        <v-text-field v-model="embed.author" label="Author" hide-details density="comfortable" />
        <v-text-field v-model="embed.color" label="Color" type="color" hide-details density="comfortable" style="max-width:120px" />
      </div>
      <div class="item-row">
        <v-text-field v-model="embed.title" label="Title" hide-details density="comfortable" />
        <v-text-field v-model="embed.url" label="Title link" placeholder="https://" persistent-placeholder hide-details density="comfortable" />
      </div>
      <v-textarea v-model="embed.description" label="Text" rows="3" auto-grow hide-details class="field-block" />
      <ImageField v-model="embed.image" label="Image" :guild-id="guildId" class="field-block" @error="e => (error = e)" />
      <ImageField v-model="embed.thumbnail" label="Thumbnail" :guild-id="guildId" class="field-block" @error="e => (error = e)" />
      <v-text-field v-model="embed.footer" label="Footer" hide-details density="comfortable" class="field-block" />
      <p v-if="embed.content" class="preview-content">{{ embed.content }}</p>
      <div class="preview" :style="{ borderColor: embed.color }">
        <small v-if="embed.author">{{ embed.author }}</small>
        <b v-if="embed.title">{{ embed.title }}</b>
        <img v-if="embed.thumbnail" :src="embed.thumbnail" alt="" class="thumb">
        <p>{{ embed.description || ' ' }}</p>
        <img v-if="embed.image" :src="embed.image" alt="">
        <small v-if="embed.footer">{{ embed.footer }}</small>
      </div>
      <div class="action-row">
        <v-select v-model="embed.channel_id" :items="textChannels" label="Channel" :loading="!guild" hide-details density="comfortable" style="max-width:260px" />
        <v-btn color="primary" :loading="sending" :disabled="!embed.channel_id" @click="sendEmbed">Post</v-btn>
      </div>
    </template>
    <p v-if="error" class="error-text small">{{ error }}</p>
  </div>
</template>

<style scoped>
.section-title { font-size: 15px; font-weight: 600; margin: 28px 0 6px }
.section-title:first-of-type { margin-top: 0 }
.item { border-top: 1px solid rgba(255, 255, 255, .06); padding: 14px 0 8px }
.item-row { display: flex; gap: 10px; margin: 0 0 14px; flex-wrap: wrap; align-items: center }
.item-row > * { flex: 1; min-width: 160px }
.preview { background: #111214; border-left: 4px solid; border-radius: 6px; padding: 12px 14px; margin: 0 0 10px; position: relative; display: flex; flex-direction: column; gap: 4px }
.preview p { margin: 0; white-space: pre-wrap }
.preview img { max-width: 100%; border-radius: 6px; margin-top: 6px }
.preview img.thumb { position: absolute; right: 12px; top: 12px; width: 64px; height: 64px; object-fit: cover; margin: 0 }
.preview small { opacity: .7 }
.preview-content { margin: 0 0 6px; white-space: pre-wrap }
</style>
