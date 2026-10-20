<script setup>
import { computed, onMounted, ref } from 'vue'
import { useDisplay } from 'vuetify'
import { mdiPlus, mdiClose, mdiChevronRight, mdiArrowUp, mdiArrowDown } from '@mdi/js'
import { api } from '../api'

const props = defineProps({ guildId: { type: String, required: true }, guild: { type: Object, default: null } })
const emit = defineEmits(['toast'])
const { xs } = useDisplay()

const panels = ref(null)
const blank = ref(null)
const error = ref('')
const edit = ref(null)
const saved = ref('')
const saving = ref(false)
const sendTo = ref(null)
const sending = ref(false)
const optEdit = ref({ open: false, index: -1, item: null })

const categories = computed(() => (props.guild?.channels || []).filter(c => c.type === 4).map(c => ({ title: c.name, value: c.id })))
const textChannels = computed(() => (props.guild?.channels || []).filter(c => c.type === 0 || c.type === 5).map(c => ({ title: `#${c.name}`, value: c.id })))
const roles = computed(() => (props.guild?.roles || []).map(r => ({ title: r.name, value: r.id })))
const dirty = computed(() => edit.value && JSON.stringify(edit.value) !== saved.value)
const colors = [
  { title: 'Blurple', value: 'primary' },
  { title: 'Grey', value: 'secondary' },
  { title: 'Green', value: 'success' },
  { title: 'Red', value: 'danger' },
]
const btnColor = { primary: '#5865F2', secondary: '#4E5058', success: '#248046', danger: '#DA373C' }

onMounted(load)
async function load() {
  try {
    const r = await api(`/api/guilds/${props.guildId}/panels`)
    panels.value = r.panels
    blank.value = r.blank
  } catch (e) { error.value = e.message }
}

function open(p) {
  edit.value = JSON.parse(JSON.stringify(p || { ...blank.value, name: panels.value.length ? `Panel ${panels.value.length + 1}` : 'Support' }))
  saved.value = p ? JSON.stringify(edit.value) : ''
  sendTo.value = null
}
function close() { edit.value = null }

async function save() {
  saving.value = true
  try {
    const p = edit.value
    const r = p.id
      ? await api(`/api/guilds/${props.guildId}/panels/${p.id}`, { method: 'PUT', body: JSON.stringify(p) })
      : await api(`/api/guilds/${props.guildId}/panels`, { method: 'POST', body: JSON.stringify(p) })
    edit.value = r.panel
    saved.value = JSON.stringify(r.panel)
    const i = panels.value.findIndex(x => x.id === r.panel.id)
    if (i < 0) panels.value.push(r.panel)
    else panels.value.splice(i, 1, r.panel)
    emit('toast', 'Panel saved')
  } catch (e) { emit('toast', e.message) }
  saving.value = false
}

async function remove() {
  if (!edit.value.id) return close()
  try {
    await api(`/api/guilds/${props.guildId}/panels/${edit.value.id}`, { method: 'DELETE' })
    panels.value = panels.value.filter(p => p.id !== edit.value.id)
    emit('toast', 'Panel deleted')
    close()
  } catch (e) { emit('toast', e.message) }
}

async function send() {
  sending.value = true
  try {
    if (dirty.value || !edit.value.id) await save()
    await api(`/api/guilds/${props.guildId}/panels/${edit.value.id}/send`, { method: 'POST', body: JSON.stringify({ channel_id: sendTo.value }) })
    emit('toast', 'Panel posted')
  } catch (e) { emit('toast', e.message) }
  sending.value = false
}

function editOption(i = -1) {
  const src = i < 0 ? { id: '', label: '', emoji: '', description: '', color: 'primary', category_id: '', staff_roles: [], ping_roles: [], questions: [], instructions: '' } : edit.value.options[i]
  optEdit.value = { open: true, index: i, item: JSON.parse(JSON.stringify(src)) }
}
function commitOption() {
  const { index, item } = optEdit.value
  if (index < 0) edit.value.options.push(item)
  else edit.value.options.splice(index, 1, item)
  optEdit.value.open = false
}
function removeOption() {
  const { index } = optEdit.value
  if (index >= 0) edit.value.options.splice(index, 1)
  optEdit.value.open = false
}
function move(i, d) {
  const o = edit.value.options
  const j = i + d
  if (j < 0 || j >= o.length) return
  ;[o[i], o[j]] = [o[j], o[i]]
}
function roleName(id) { return props.guild?.roles?.find(r => r.id === id)?.name || id }
function categoryName(id) { return props.guild?.channels?.find(c => c.id === id)?.name }
function optionMeta(o) {
  const bits = []
  if (o.questions.length) bits.push(`${o.questions.length} question${o.questions.length > 1 ? 's' : ''}`)
  if (o.category_id && categoryName(o.category_id)) bits.push(categoryName(o.category_id))
  if (o.ping_roles.length) bits.push(`pings ${o.ping_roles.map(roleName).join(', ')}`)
  if (o.instructions) bits.push('AI instructions')
  return bits.join(' · ') || o.description || 'Default settings'
}
</script>

<template>
  <p v-if="error" class="muted">{{ error }}</p>
  <div v-else-if="!panels" class="row-list"><div v-for="i in 3" :key="i" class="skeleton-row"><span /></div></div>

  <div v-else-if="!edit" class="agent">
    <div class="section-head">
      <div><h2 class="h2">Panels</h2><p class="muted small">The message members click to open a ticket. Each ticket type can go to its own category, ping its own roles and ask its own questions.</p></div>
      <v-btn variant="text" color="secondary" :prepend-icon="mdiPlus" @click="open()">New</v-btn>
    </div>
    <div v-if="!panels.length" class="empty-state">
      <h2>No panels yet</h2>
      <p class="muted">Without one, /panel posts a single "Open a ticket" button.</p>
      <v-btn color="primary" @click="open()">Create a panel</v-btn>
    </div>
    <div v-else class="row-list">
      <button v-for="p in panels" :key="p.id" class="file-row edit-row" @click="open(p)">
        <div class="file-main"><span>{{ p.name }}</span><span class="muted small ellipsis">{{ p.options.map(o => o.label).join(', ') }}</span></div>
        <v-icon :icon="mdiChevronRight" size="20" class="muted" />
      </button>
    </div>
  </div>

  <div v-else class="panel-editor">
    <div class="panel-form">
      <div class="section-head">
        <h2 class="h2">{{ edit.id ? 'Edit panel' : 'New panel' }}</h2>
        <v-btn variant="text" @click="close">Back</v-btn>
      </div>
      <v-text-field v-model="edit.name" label="Name" hint="For you and /panel name:..." persistent-hint class="field-block" />
      <v-text-field v-model="edit.title" label="Title" counter="256" class="field-block" />
      <v-textarea v-model="edit.description" label="Message" auto-grow rows="3" counter="4000" class="field-block" />
      <div class="two-col">
        <v-text-field v-model="edit.color" label="Color" placeholder="#5865F2">
          <template #prepend-inner><input v-model="edit.color" type="color" class="color-dot" aria-label="Pick color"></template>
        </v-text-field>
        <v-select v-model="edit.style" :items="[{ title: 'Buttons', value: 'buttons' }, { title: 'Dropdown', value: 'select' }]" label="Style" />
      </div>
      <v-text-field v-if="edit.style === 'select'" v-model="edit.placeholder" label="Dropdown text" placeholder="Pick what you need help with" class="field-block" />
      <v-text-field v-model="edit.image" label="Image URL (optional)" placeholder="https://..." class="field-block" />

      <div class="section-head" style="margin-top:8px">
        <div><h3>Ticket types</h3><p class="muted small">Up to 25. Click one to set its category, roles, questions and AI instructions.</p></div>
        <v-btn variant="text" color="secondary" :prepend-icon="mdiPlus" :disabled="edit.options.length >= 25" @click="editOption()">Add</v-btn>
      </div>
      <div class="row-list">
        <div v-for="(o, i) in edit.options" :key="o.id || i" class="file-row">
          <button class="file-main opt-main" @click="editOption(i)">
            <span>{{ o.emoji ? o.emoji + ' ' : '' }}{{ o.label }}</span>
            <span class="muted small ellipsis">{{ optionMeta(o) }}</span>
          </button>
          <v-btn variant="text" size="small" :icon="mdiArrowUp" :disabled="i === 0" aria-label="Move up" @click="move(i, -1)" />
          <v-btn variant="text" size="small" :icon="mdiArrowDown" :disabled="i === edit.options.length - 1" aria-label="Move down" @click="move(i, 1)" />
        </div>
      </div>

      <div class="action-row sticky-save">
        <v-btn color="primary" :loading="saving" :disabled="(!dirty && !!edit.id) || !edit.options.length" @click="save">Save</v-btn>
        <v-btn v-if="edit.id" variant="text" color="error" @click="remove">Delete</v-btn>
        <span v-if="dirty" class="muted small">Unsaved changes</span>
      </div>

      <section class="agent-section">
        <h3>Post it</h3>
        <p class="muted small">Or run <code>/panel name:{{ edit.name }}</code> in Discord.</p>
        <div class="send-row">
          <v-select v-model="sendTo" :items="textChannels" label="Channel" hide-details density="comfortable" />
          <v-btn color="primary" :loading="sending" :disabled="!sendTo || !edit.options.length" @click="send">Post</v-btn>
        </div>
      </section>
    </div>

    <aside class="discord-preview" aria-label="Preview">
      <p class="muted small">Preview</p>
      <div class="dc-msg">
        <div class="dc-embed" :style="{ borderColor: edit.color }">
          <strong v-if="edit.title">{{ edit.title }}</strong>
          <p v-if="edit.description">{{ edit.description }}</p>
          <img v-if="edit.image" :src="edit.image" alt="">
        </div>
        <div v-if="edit.style === 'select'" class="dc-select">{{ edit.placeholder || 'Pick what you need help with' }}</div>
        <div v-else class="dc-buttons">
          <span v-for="o in edit.options" :key="o.id || o.label" class="dc-btn" :style="{ background: btnColor[o.color] || btnColor.primary }">{{ o.emoji ? o.emoji + ' ' : '' }}{{ o.label || 'Untitled' }}</span>
        </div>
      </div>
    </aside>

    <v-dialog v-model="optEdit.open" :fullscreen="xs" max-width="640" scrollable>
      <v-card v-if="optEdit.item" class="editor">
        <div class="editor-head">
          <h2>{{ optEdit.index < 0 ? 'New' : 'Edit' }} ticket type</h2>
          <v-btn variant="text" :icon="mdiClose" aria-label="Close" @click="optEdit.open = false" />
        </div>
        <v-card-text class="editor-body">
          <div class="two-col">
            <v-text-field v-model="optEdit.item.label" label="Label" counter="80" placeholder="Billing" />
            <v-text-field v-model="optEdit.item.emoji" label="Emoji (optional)" placeholder="<:name:id> or unicode" />
          </div>
          <div class="two-col">
            <v-select v-if="edit.style === 'buttons'" v-model="optEdit.item.color" :items="colors" label="Button color" />
            <v-text-field v-else v-model="optEdit.item.description" label="Description" counter="100" />
            <v-select v-model="optEdit.item.category_id" :items="categories" label="Category" clearable hint="Default: server ticket category" persistent-hint />
          </div>
          <v-autocomplete v-model="optEdit.item.staff_roles" :items="roles" label="Support roles" multiple chips closable-chips hint="Can see and handle these tickets, on top of server staff" persistent-hint class="field-block" />
          <v-autocomplete v-model="optEdit.item.ping_roles" :items="roles" label="Ping on open" multiple chips closable-chips hint="Also pinged when the AI escalates" persistent-hint class="field-block" />

          <div class="sub-head"><h3>Questions</h3><v-btn size="small" variant="text" color="secondary" :prepend-icon="mdiPlus" :disabled="optEdit.item.questions.length >= 5" @click="optEdit.item.questions.push({ label: '', placeholder: '', long: false, required: true })">Add</v-btn></div>
          <p class="muted small">Asked in a popup before the ticket opens (max 5). The AI reads the answers.</p>
          <div v-for="(q, i) in optEdit.item.questions" :key="i" class="question">
            <v-text-field v-model="q.label" label="Question" counter="45" hide-details="auto" density="compact" />
            <v-text-field v-model="q.placeholder" label="Placeholder" hide-details density="compact" />
            <div class="q-flags">
              <v-checkbox-btn v-model="q.long" label="Long answer" />
              <v-checkbox-btn v-model="q.required" label="Required" />
              <v-btn variant="text" size="small" :icon="mdiClose" aria-label="Remove" @click="optEdit.item.questions.splice(i, 1)" />
            </div>
          </div>

          <v-textarea v-model="optEdit.item.instructions" label="AI instructions for this type (optional)" auto-grow rows="3" counter="4000" placeholder="Billing tickets: always ask for the order ID and email first." class="field-block" style="margin-top:12px" />
        </v-card-text>
        <div class="editor-actions">
          <v-btn v-if="optEdit.index >= 0" variant="text" color="error" @click="removeOption">Delete</v-btn>
          <v-spacer />
          <v-btn variant="text" @click="optEdit.open = false">Cancel</v-btn>
          <v-btn color="primary" :disabled="!optEdit.item.label.trim()" @click="commitOption">Done</v-btn>
        </div>
      </v-card>
    </v-dialog>
  </div>
</template>
