<script setup>
import { computed, onMounted, ref } from 'vue'
import { api } from '../api'

const props = defineProps({ guildId: String, guild: Object })
const emit = defineEmits(['toast'])
const cfg = ref(null)
const words = ref('')
const domains = ref('')
const saved = ref('')
const saving = ref(false)
const error = ref('')
const roles = computed(() => (props.guild?.roles || []).map(r => ({ title: r.name, value: r.id })))
const textChannels = computed(() => (props.guild?.channels || []).filter(c => c.type === 0 || c.type === 5).map(c => ({ title: `#${c.name}`, value: c.id })))
const actions = [{ title: 'Delete only', value: 'delete' }, { title: 'Delete and warn', value: 'warn' }, { title: 'Delete and timeout', value: 'timeout' }, { title: 'Delete and kick', value: 'kick' }, { title: 'Delete and ban', value: 'ban' }]
const rules = [
  { key: 'spam', title: 'Spam', text: 'Too many messages in 5 seconds', limit: 'Messages' },
  { key: 'duplicates', title: 'Repeated messages', text: 'The same message over and over in a minute', limit: 'Repeats' },
  { key: 'scam', title: 'Scam links', text: 'Fake Nitro, phishing and known scam sites. The list updates itself' },
  { key: 'mentions', title: 'Mass mentions', text: 'Pinging lots of people or roles at once', limit: 'Mentions' },
  { key: 'invites', title: 'Invite links', text: 'Invites to other servers. Your own server is allowed' },
  { key: 'links', title: 'All links', text: 'Any link outside the allowed sites below' },
  { key: 'words', title: 'Blocked words', text: 'Catches l33t spellings. Use * as a wildcard' },
  { key: 'caps', title: 'Caps', text: 'Mostly capital letters', limit: '% caps' },
  { key: 'emoji', title: 'Emoji spam', text: 'Lots of emojis in one message', limit: 'Emojis' },
  { key: 'zalgo', title: 'Glitched text', text: 'Zalgo text that covers other messages' },
]
const lines = s => s.split('\n').map(x => x.trim()).filter(Boolean)
const body = () => ({ ...cfg.value, word_list: lines(words.value), allowed_domains: lines(domains.value), raid: { ...cfg.value.raid, alert_channel_id: cfg.value.raid.alert_channel_id || '' } })
const dirty = computed(() => cfg.value && JSON.stringify(body()) !== saved.value)

function load(c) {
  cfg.value = { ...c, raid: { ...c.raid, alert_channel_id: c.raid.alert_channel_id || null } }
  words.value = c.word_list.join('\n')
  domains.value = c.allowed_domains.join('\n')
  saved.value = JSON.stringify(body())
}

onMounted(async () => {
  try { load((await api(`/api/guilds/${props.guildId}/modules/automod`)).config) } catch (e) { error.value = e.message }
})

async function save() {
  saving.value = true
  error.value = ''
  try {
    load((await api(`/api/guilds/${props.guildId}/modules/automod`, { method: 'PUT', body: JSON.stringify(body()) })).config)
    emit('toast', 'Automod saved')
  } catch (e) { error.value = e.message }
  saving.value = false
}
</script>

<template>
  <div class="settings-form">
    <div v-if="!cfg && !error" class="row-list"><div v-for="i in 4" :key="i" class="skeleton-row"><span /></div></div>
    <template v-if="cfg">
      <div class="switch-row">
        <div><h3>Automod</h3><p class="muted small">Staff, admins and the roles you ignore are never touched</p></div>
        <v-switch v-model="cfg.enabled" aria-label="Automod" />
      </div>
      <div :class="{ off: !cfg.enabled }">
        <div v-for="r in rules" :key="r.key" class="rule">
          <div class="switch-row">
            <div><h3>{{ r.title }}</h3><p class="muted small">{{ r.text }}</p></div>
            <v-switch v-model="cfg[r.key].on" :aria-label="r.title" />
          </div>
          <div v-if="cfg[r.key].on" class="rule-opts">
            <v-select v-model="cfg[r.key].action" :items="actions" label="Action" hide-details density="comfortable" style="max-width:210px" />
            <v-text-field v-if="cfg[r.key].action === 'timeout'" v-model.number="cfg[r.key].minutes" type="number" min="1" label="Minutes" hide-details density="comfortable" style="max-width:120px" />
            <v-text-field v-if="r.limit" v-model.number="cfg[r.key].limit" type="number" :label="r.limit" hide-details density="comfortable" style="max-width:120px" />
          </div>
          <v-textarea v-if="r.key === 'words' && cfg.words.on" v-model="words" label="Words, one per line" rows="3" auto-grow hide-details class="rule-text" />
          <v-textarea v-if="r.key === 'links' && cfg.links.on" v-model="domains" label="Allowed sites, one per line" rows="3" auto-grow hide-details class="rule-text" />
        </div>
        <v-select v-model="cfg.ignored_roles" :items="roles" label="Ignored roles" multiple chips closable-chips :loading="!guild" class="field-block" style="margin-top:20px" />
        <v-select v-model="cfg.ignored_channels" :items="textChannels" label="Ignored channels" multiple chips closable-chips :loading="!guild" class="field-block" />

        <section class="settings-section">
          <h3>Anti-raid</h3>
          <p class="muted small">Needs the Server Members intent on Kev. Alerts go to the mod log unless you pick a channel.</p>
          <div class="switch-row">
            <div><h3>Lock down on join spikes</h3><p class="muted small">Raises verification to the highest level, with an End lockdown button</p></div>
            <v-switch v-model="cfg.raid.on" aria-label="Lock down on join spikes" />
          </div>
          <div v-if="cfg.raid.on" class="rule-opts">
            <v-text-field v-model.number="cfg.raid.joins" type="number" label="Joins" hide-details density="comfortable" style="max-width:100px" />
            <v-text-field v-model.number="cfg.raid.seconds" type="number" label="Within seconds" hide-details density="comfortable" style="max-width:140px" />
            <v-text-field v-model.number="cfg.raid.lockdown_minutes" type="number" label="Lockdown minutes" hide-details density="comfortable" style="max-width:160px" />
          </div>
          <div v-if="cfg.raid.on" class="switch-row">
            <div><h3>Kick joins during a lockdown</h3><p class="muted small">Stops the raid completely, real people can rejoin later</p></div>
            <v-switch v-model="cfg.raid.kick_during" aria-label="Kick joins during a lockdown" />
          </div>
          <div class="rule-opts">
            <v-text-field v-model.number="cfg.raid.min_account_days" type="number" min="0" label="Minimum account age (days)" hint="0 is off" persistent-hint density="comfortable" style="max-width:240px" />
            <v-select v-if="cfg.raid.min_account_days > 0" v-model="cfg.raid.young_action" :items="[{ title: 'Kick', value: 'kick' }, { title: 'Timeout until old enough', value: 'timeout' }, { title: 'Ban', value: 'ban' }]" label="Newer accounts" density="comfortable" hide-details style="max-width:240px" />
          </div>
          <v-select v-model="cfg.raid.alert_channel_id" :items="textChannels" label="Alert channel" clearable :loading="!guild" class="field-block" style="margin-top:16px" />
        </section>
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
.rule { border-top: 1px solid rgba(255, 255, 255, .06) }
.rule-opts { display: flex; gap: 10px; flex-wrap: wrap; align-items: center; padding: 0 0 14px }
.rule-text { padding: 0 0 14px }
.off { opacity: .45; transition: opacity .25s ease }
</style>
