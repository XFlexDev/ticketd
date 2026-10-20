<script setup>
import { computed, nextTick, onMounted, ref } from 'vue'
import { useDisplay } from 'vuetify'
import { mdiPlus, mdiClose, mdiChevronRight, mdiSend } from '@mdi/js'
import { api } from '../api'

const props = defineProps({ guildId: { type: String, required: true } })
const emit = defineEmits(['toast'])
const { xs } = useDisplay()

const cfg = ref(null)
const advanced = ref(false)
const tones = [{ title: 'Friendly', value: '' }, { title: 'Professional', value: 'professional' }, { title: 'Casual', value: 'casual' }, { title: 'Short', value: 'short' }]
const builtins = [
  ['Ticket info', 'Ticket type, priority, member roles and join date'],
  ['Past tickets', "The member's earlier tickets here"],
  ['Server info', 'Member count, roles and public channels'],
  ['Read web pages', 'Your site, docs or status page'],
  ['Time', 'Current date and time'],
  ['Set priority', 'Low to urgent'],
  ['Rename ticket', 'Short topic once it is clear'],
  ['Staff notes', 'Private notes in the log channel'],
  ['Escalate', 'Asks first, then pings staff'],
]
const saved = ref('')
const saving = ref(false)
const error = ref('')
const dirty = computed(() => cfg.value && JSON.stringify(cfg.value) !== saved.value)

const templates = [
  { name: 'Game server', text: 'We run a [game] server ([server IP]). Help players with joining problems, store purchases and ranks, bug reports and ban appeals.\n\nFor connection problems, ask for their username, edition or platform, and the exact error.\nBan appeals and payment problems always go to staff.' },
  { name: 'Shop', text: 'We sell [products] at [website]. Help customers with orders, shipping, returns and product questions.\n\nAlways ask for the order number and the email used.\nRefunds, chargebacks and damaged items go to staff.' },
  { name: 'App or software', text: 'We make [app name]. Help users with bugs, login and account problems, and how-to questions.\n\nFor bugs, ask for device, OS, app version and steps to reproduce.\nAccount deletion and billing go to staff.' },
  { name: 'Community', text: 'This is the [name] community. Answer questions about the rules, roles and events, and take reports about other members.\n\nFor reports, ask who, what happened and for screenshots or message links.\nReports about members always go to staff.' },
]

onMounted(async () => {
  try {
    const r = await api(`/api/guilds/${props.guildId}/agent`)
    cfg.value = r.config
    saved.value = JSON.stringify(cfg.value)
  } catch (e) { error.value = e.message }
})

async function save() {
  saving.value = true
  try {
    const r = await api(`/api/guilds/${props.guildId}/agent`, { method: 'PUT', body: JSON.stringify(cfg.value) })
    cfg.value = r.config
    saved.value = JSON.stringify(cfg.value)
    emit('toast', 'Agent saved')
  } catch (e) { emit('toast', e.message) }
  saving.value = false
}

const lists = { skill: 'skills', webhook: 'webhooks', mcp: 'mcp' }
const titles = { skill: 'Skill', webhook: 'Webhook', mcp: 'MCP server' }
const blank = {
  skill: () => ({ name: '', description: '', content: '' }),
  webhook: () => ({ name: '', description: '', method: 'POST', url: '', headers: [], params: [] }),
  mcp: () => ({ name: '', url: '', headers: [] }),
}
const editor = ref({ open: false, kind: 'skill', index: -1, item: blank.skill() })
const mcpTools = ref(null)
const mcpError = ref('')
const mcpTesting = ref(false)

function edit(kind, index = -1) {
  const src = index < 0 ? blank[kind]() : cfg.value[lists[kind]][index]
  editor.value = { open: true, kind, index, item: JSON.parse(JSON.stringify(src)) }
  mcpTools.value = null
  mcpError.value = ''
}
const canCommit = computed(() => {
  const { kind, item } = editor.value
  if (!item.name.trim()) return false
  if (kind === 'skill') return !!item.content.trim()
  return /^https?:\/\/\S+$/.test(item.url.trim())
})
function commit() {
  const { kind, index, item } = editor.value
  const list = cfg.value[lists[kind]]
  if (index < 0) list.push(item)
  else list.splice(index, 1, item)
  editor.value.open = false
}
function removeItem() {
  const { kind, index } = editor.value
  if (index >= 0) cfg.value[lists[kind]].splice(index, 1)
  editor.value.open = false
}
async function testMcp() {
  mcpTesting.value = true
  mcpTools.value = null
  mcpError.value = ''
  try {
    const r = await api(`/api/guilds/${props.guildId}/agent/mcp-test`, { method: 'POST', body: JSON.stringify(editor.value.item) })
    mcpTools.value = r.tools
  } catch (e) { mcpError.value = e.message }
  mcpTesting.value = false
}
function host(url) { try { return new URL(url).host } catch { return url } }

const chat = ref([])
const draft = ref('')
const thinking = ref(false)
const chatEl = ref(null)
async function scrollChat() { await nextTick(); chatEl.value?.scrollTo({ top: chatEl.value.scrollHeight, behavior: 'smooth' }) }
async function send() {
  const text = draft.value.trim()
  if (!text || thinking.value) return
  chat.value.push({ role: 'user', content: text })
  draft.value = ''
  thinking.value = true
  scrollChat()
  try {
    const messages = chat.value.filter(m => m.role !== 'event').map(({ role, content }) => ({ role, content }))
    const r = await api(`/api/guilds/${props.guildId}/agent/test`, { method: 'POST', body: JSON.stringify({ config: cfg.value, messages }) })
    for (const t of r.tools) chat.value.push({ role: 'event', content: `Used ${t.name}` })
    if (r.reply) chat.value.push({ role: 'assistant', content: r.reply })
    for (const a of r.actions) chat.value.push({ role: 'event', content: a.type === 'escalate' ? `Pings staff: ${a.reason || 'no reason'}` : 'Closes the ticket' })
  } catch (e) { chat.value.push({ role: 'event', content: e.message }) }
  thinking.value = false
  scrollChat()
}
</script>

<template>
  <p v-if="error" class="muted">{{ error }}</p>
  <div v-else-if="!cfg" class="row-list"><div v-for="i in 3" :key="i" class="skeleton-row"><span /></div></div>
  <div v-else class="agent">
    <section class="agent-section">
      <h2>Instructions</h2>
      <p class="muted small">Tell the AI what your server is about, what it should help with and when to get staff.</p>
      <div v-if="!cfg.instructions" class="template-row">
        <span class="muted small">Start from</span>
        <v-btn v-for="t in templates" :key="t.name" size="small" variant="tonal" @click="cfg.instructions = t.text">{{ t.name }}</v-btn>
      </div>
      <v-textarea v-model="cfg.instructions" auto-grow rows="5" counter="8000" placeholder="We run a Minecraft server. Help players with joining issues and store purchases. Ban appeals go to staff." class="field-block" />
      <div class="switch-row">
        <div><h3>Close solved tickets</h3><p class="muted small">Asks the member first</p></div>
        <v-switch v-model="cfg.can_close" aria-label="Close solved tickets" />
      </div>
    </section>

    <section class="agent-section">
      <div class="section-head">
        <div><h2>Skills</h2><p class="muted small">Guides the AI loads when a topic comes up: refund policy, setup steps, FAQs.</p></div>
        <v-btn variant="text" color="secondary" :prepend-icon="mdiPlus" @click="edit('skill')">Add</v-btn>
      </div>
      <div v-if="cfg.skills.length" class="row-list">
        <button v-for="(s, i) in cfg.skills" :key="i" class="file-row edit-row" @click="edit('skill', i)">
          <div class="file-main"><span>{{ s.name }}</span><span class="muted small ellipsis">{{ s.description }}</span></div>
          <v-icon :icon="mdiChevronRight" size="20" class="muted" />
        </button>
      </div>
    </section>

    <section class="agent-section">
      <div class="section-head">
        <div><h2>Webhooks</h2><p class="muted small">Your own endpoints the AI can call, like looking up an order or resetting a password.</p></div>
        <v-btn variant="text" color="secondary" :prepend-icon="mdiPlus" @click="edit('webhook')">Add</v-btn>
      </div>
      <div v-if="cfg.webhooks.length" class="row-list">
        <button v-for="(w, i) in cfg.webhooks" :key="i" class="file-row edit-row" @click="edit('webhook', i)">
          <div class="file-main"><span>{{ w.name }}</span><span class="muted small mono ellipsis">{{ w.method }} {{ host(w.url) }}</span></div>
          <v-icon :icon="mdiChevronRight" size="20" class="muted" />
        </button>
      </div>
    </section>

    <section class="agent-section">
      <div class="section-head">
        <div><h2>MCP servers</h2><p class="muted small">Connect any MCP server over HTTP and the AI gets all its tools.</p></div>
        <v-btn variant="text" color="secondary" :prepend-icon="mdiPlus" @click="edit('mcp')">Add</v-btn>
      </div>
      <div v-if="cfg.mcp.length" class="row-list">
        <button v-for="(m, i) in cfg.mcp" :key="i" class="file-row edit-row" @click="edit('mcp', i)">
          <div class="file-main"><span>{{ m.name }}</span><span class="muted small mono ellipsis">{{ host(m.url) }}</span></div>
          <v-icon :icon="mdiChevronRight" size="20" class="muted" />
        </button>
      </div>
    </section>

    <section class="agent-section">
      <h2>Built-in tools</h2>
      <p class="muted small">Always on.</p>
      <div class="builtin-list">
        <div v-for="b in builtins" :key="b[0]" class="tool"><span>{{ b[0] }}</span><span class="muted small">{{ b[1] }}</span></div>
      </div>
    </section>

    <section class="agent-section">
      <button class="advanced-toggle" :aria-expanded="advanced" @click="advanced = !advanced">
        <h2>Advanced</h2>
        <v-icon :icon="mdiChevronRight" size="20" :class="['muted', 'chev', { open: advanced }]" />
      </button>
      <v-expand-transition>
        <div v-show="advanced" class="advanced">
          <div class="two-col">
            <v-select v-model="cfg.tone" :items="tones" label="Tone" />
            <v-text-field v-model="cfg.language" label="Language" placeholder="Match the member" hint="Leave empty to reply in the member's language" persistent-hint />
          </div>
          <div class="slider-row">
            <div><h3>Creativity</h3><p class="muted small">{{ cfg.temperature <= 0.3 ? 'Precise' : cfg.temperature <= 0.8 ? 'Balanced' : 'Creative' }}</p></div>
            <v-slider v-model="cfg.temperature" :min="0" :max="1.5" :step="0.1" thumb-label hide-details color="primary" />
          </div>
          <div class="slider-row">
            <div><h3>Reply length</h3><p class="muted small">Up to {{ cfg.max_reply }} characters</p></div>
            <v-slider v-model="cfg.max_reply" :min="200" :max="1900" :step="100" hide-details color="primary" />
          </div>
          <div class="slider-row">
            <div><h3>Memory</h3><p class="muted small">Last {{ cfg.history }} messages</p></div>
            <v-slider v-model="cfg.history" :min="2" :max="50" :step="1" hide-details color="primary" />
          </div>
          <div class="slider-row">
            <div><h3>Tool steps</h3><p class="muted small">Up to {{ cfg.max_steps }} rounds of tools per reply</p></div>
            <v-slider v-model="cfg.max_steps" :min="1" :max="10" :step="1" hide-details color="primary" />
          </div>
          <div class="slider-row">
            <div><h3>Reply delay</h3><p class="muted small">{{ cfg.reply_delay ? `Waits ${cfg.reply_delay}s so members can finish typing` : 'Answers right away' }}</p></div>
            <v-slider v-model="cfg.reply_delay" :min="0" :max="30" :step="1" hide-details color="primary" />
          </div>
          <v-textarea v-model="cfg.greeting" label="Fixed greeting (optional)" auto-grow rows="2" counter="1500" placeholder="Hey {user}! Thanks for opening a {type} ticket. What's going on?" hint="{user} mentions the member, {type} is the ticket type. Empty = the AI writes one" persistent-hint class="field-block" />
        </div>
      </v-expand-transition>
    </section>

    <div class="action-row sticky-save">
      <v-btn color="primary" :loading="saving" :disabled="!dirty" @click="save">Save</v-btn>
      <span v-if="dirty" class="muted small">Unsaved changes</span>
    </div>

    <section class="agent-section">
      <div class="section-head">
        <div><h2>Try it</h2><p class="muted small">Chat with your AI using the setup above, saved or not. Staff pings and closing are only shown here.</p></div>
        <v-btn v-if="chat.length" variant="text" @click="chat = []">Reset</v-btn>
      </div>
      <div ref="chatEl" class="chat">
        <p v-if="!chat.length" class="muted small chat-empty">Write like a member opening a ticket.</p>
        <div v-for="(m, i) in chat" :key="i" :class="['msg', m.role]">{{ m.content }}</div>
        <div v-if="thinking" class="msg assistant typing"><span /><span /><span /></div>
      </div>
      <form class="chat-input" @submit.prevent="send">
        <v-text-field v-model="draft" placeholder="My order never arrived" hide-details density="comfortable" :disabled="thinking" />
        <v-btn type="submit" color="primary" :icon="mdiSend" :disabled="!draft.trim() || thinking" aria-label="Send" />
      </form>
    </section>

    <v-dialog v-model="editor.open" :fullscreen="xs" max-width="640" scrollable>
      <v-card class="editor">
        <div class="editor-head">
          <h2>{{ editor.index < 0 ? 'New' : 'Edit' }} {{ titles[editor.kind].toLowerCase() }}</h2>
          <v-btn variant="text" :icon="mdiClose" aria-label="Close" @click="editor.open = false" />
        </div>
        <v-card-text class="editor-body">
          <v-text-field v-model="editor.item.name" label="Name" :placeholder="editor.kind === 'webhook' ? 'lookup_order' : editor.kind === 'mcp' ? 'docs' : 'refunds'" class="field-block" />
          <template v-if="editor.kind === 'skill'">
            <v-text-field v-model="editor.item.description" label="When to use it" placeholder="Someone asks for a refund or a chargeback" class="field-block" />
            <v-textarea v-model="editor.item.content" label="Guide" auto-grow rows="6" counter="12000" placeholder="Refunds are possible within 14 days. Ask for the order number, then..." class="field-block" />
          </template>
          <template v-else-if="editor.kind === 'webhook'">
            <v-text-field v-model="editor.item.description" label="What it does" placeholder="Looks up an order's status by order number" class="field-block" />
            <div class="url-row">
              <v-select v-model="editor.item.method" :items="['POST', 'GET']" label="Method" class="method" />
              <v-text-field v-model="editor.item.url" label="URL" placeholder="https://api.example.com/orders" />
            </div>
            <div class="sub-head"><h3>Inputs</h3><v-btn size="small" variant="text" color="secondary" :prepend-icon="mdiPlus" @click="editor.item.params.push({ name: '', description: '', required: true })">Add</v-btn></div>
            <p class="muted small">The AI fills these in. POST sends them as JSON with the ticket info, GET as query parameters.</p>
            <div v-for="(p, i) in editor.item.params" :key="i" class="kv-row">
              <v-text-field v-model="p.name" label="Name" placeholder="order_id" hide-details density="compact" />
              <v-text-field v-model="p.description" label="Description" hide-details density="compact" />
              <v-checkbox-btn v-model="p.required" label="Required" />
              <v-btn variant="text" size="small" :icon="mdiClose" aria-label="Remove" @click="editor.item.params.splice(i, 1)" />
            </div>
          </template>
          <template v-else>
            <v-text-field v-model="editor.item.url" label="URL" placeholder="https://mcp.example.com/mcp" hint="Streamable HTTP endpoint" persistent-hint class="field-block" />
          </template>
          <template v-if="editor.kind !== 'skill'">
            <div class="sub-head"><h3>Headers</h3><v-btn size="small" variant="text" color="secondary" :prepend-icon="mdiPlus" @click="editor.item.headers.push({ name: 'Authorization', value: '' })">Add</v-btn></div>
            <div v-for="(h, i) in editor.item.headers" :key="i" class="kv-row two">
              <v-text-field v-model="h.name" label="Header" hide-details density="compact" />
              <v-text-field v-model="h.value" label="Value" type="password" autocomplete="off" hide-details density="compact" />
              <v-btn variant="text" size="small" :icon="mdiClose" aria-label="Remove" @click="editor.item.headers.splice(i, 1)" />
            </div>
          </template>
          <template v-if="editor.kind === 'mcp'">
            <div class="action-row"><v-btn variant="tonal" :loading="mcpTesting" :disabled="!canCommit" @click="testMcp">Test connection</v-btn></div>
            <p v-if="mcpError" class="error-text small">{{ mcpError }}</p>
            <div v-if="mcpTools" class="tool-list">
              <p class="muted small">{{ mcpTools.length }} tools</p>
              <div v-for="t in mcpTools" :key="t.name" class="tool"><span class="mono">{{ t.name }}</span><span class="muted small">{{ t.description }}</span></div>
            </div>
          </template>
        </v-card-text>
        <div class="editor-actions">
          <v-btn v-if="editor.index >= 0" variant="text" color="error" @click="removeItem">Delete</v-btn>
          <v-spacer />
          <v-btn variant="text" @click="editor.open = false">Cancel</v-btn>
          <v-btn color="primary" :disabled="!canCommit" @click="commit">Done</v-btn>
        </div>
      </v-card>
    </v-dialog>
  </div>
</template>
