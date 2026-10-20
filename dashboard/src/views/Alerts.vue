<script setup>
import { computed, onMounted, ref } from 'vue'
import { api } from '../api'

const props = defineProps({ guildId: String, guild: Object })
const emit = defineEmits(['toast'])
const cfg = ref(null)
const saved = ref('')
const saving = ref(false)
const error = ref('')
const tests = ref({})
const roles = computed(() => (props.guild?.roles || []).map(r => ({ title: r.name, value: r.id })))
const textChannels = computed(() => (props.guild?.channels || []).filter(c => c.type === 0 || c.type === 5).map(c => ({ title: `#${c.name}`, value: c.id })))
const kinds = { youtube: { title: 'YouTube', label: 'Channel', hint: '@handle or channel link' }, reddit: { title: 'Reddit', label: 'Subreddit', hint: 'like gaming or r/gaming' }, rss: { title: 'RSS feed', label: 'Feed link', hint: 'any RSS or Atom link, blogs, news, Steam, GitHub releases' } }
const defaults = { youtube: '**{author}** posted a new video: {title}\n{link}', reddit: 'New post in **{feed}**: {title}\n{link}', rss: '**{feed}**: {title}\n{link}' }
const body = () => ({ feeds: cfg.value.feeds.map(f => ({ ...f, channel_id: f.channel_id || '', role_id: f.role_id || '' })) })
const dirty = computed(() => cfg.value && JSON.stringify(body()) !== saved.value)

function load(c) {
  cfg.value = { feeds: c.feeds.map(f => ({ ...f, channel_id: f.channel_id || null, role_id: f.role_id || null })) }
  saved.value = JSON.stringify(body())
}
onMounted(async () => {
  try { load((await api(`/api/guilds/${props.guildId}/modules/alerts`)).config) } catch (e) { error.value = e.message }
})
function add(kind) {
  cfg.value.feeds.push({ id: '', kind, source: '', channel_id: null, role_id: null, message: '' })
}
async function test(i) {
  tests.value[i] = { loading: true }
  try {
    const r = await api(`/api/guilds/${props.guildId}/alerts/test`, { method: 'POST', body: JSON.stringify(body().feeds[i]) })
    tests.value[i] = { name: r.name, items: r.items }
  } catch (e) { tests.value[i] = { error: e.message } }
}
async function save() {
  saving.value = true
  error.value = ''
  try {
    load((await api(`/api/guilds/${props.guildId}/modules/alerts`, { method: 'PUT', body: JSON.stringify(body()) })).config)
    emit('toast', 'Alerts saved')
  } catch (e) { error.value = e.message }
  saving.value = false
}
</script>

<template>
  <div class="settings-form">
    <div v-if="!cfg && !error" class="row-list"><div v-for="i in 3" :key="i" class="skeleton-row"><span /></div></div>
    <template v-if="cfg">
      <p class="muted small" style="margin:0 0 20px">Kev checks every 5 minutes and posts anything new. Old posts are skipped when you add an alert. Messages can use <code>{title}</code> <code>{link}</code> <code>{author}</code> <code>{feed}</code>.</p>
      <p v-if="!cfg.feeds.length" class="muted small">No alerts yet.</p>
      <div v-for="(f, i) in cfg.feeds" :key="i" class="alert">
        <div class="alert-head">
          <b>{{ kinds[f.kind].title }}</b>
          <v-btn variant="text" size="small" @click="cfg.feeds.splice(i, 1); tests = {}">Remove</v-btn>
        </div>
        <v-text-field v-model="f.source" :label="kinds[f.kind].label" :hint="kinds[f.kind].hint" persistent-hint class="field-block" />
        <div class="alert-row">
          <v-select v-model="f.channel_id" :items="textChannels" label="Post in" :loading="!guild" hide-details density="comfortable" />
          <v-select v-model="f.role_id" :items="roles" label="Ping role" clearable :loading="!guild" hide-details density="comfortable" />
        </div>
        <v-textarea v-model="f.message" label="Message" :placeholder="defaults[f.kind]" persistent-placeholder rows="2" auto-grow hide-details class="field-block" />
        <div class="action-row" style="margin-top:8px">
          <v-btn variant="tonal" size="small" :loading="tests[i]?.loading" :disabled="!f.source" @click="test(i)">Test</v-btn>
          <span v-if="tests[i]?.error" class="error-text small">{{ tests[i].error }}</span>
          <span v-else-if="tests[i]?.items" class="muted small">{{ tests[i].name }}: {{ tests[i].items[0] || 'no posts yet' }}</span>
        </div>
      </div>
      <div class="action-row">
        <v-btn v-for="(k, key) in kinds" :key="key" variant="tonal" size="small" :disabled="cfg.feeds.length >= 25" @click="add(key)">Add {{ k.title }}</v-btn>
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
.alert { border-top: 1px solid rgba(255, 255, 255, .06); padding: 14px 0 18px }
.alert-head { display: flex; justify-content: space-between; align-items: center; margin: 0 0 10px }
.alert-row { display: flex; gap: 10px; margin: 0 0 16px; flex-wrap: wrap }
.alert-row > * { flex: 1; min-width: 180px }
</style>
