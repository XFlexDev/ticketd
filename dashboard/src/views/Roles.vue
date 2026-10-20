<script setup>
import { computed, onMounted, ref } from 'vue'
import { api } from '../api'

const props = defineProps({ guildId: String, guild: Object })
const emit = defineEmits(['toast'])
const cfg = ref(null)
const saved = ref('')
const saving = ref(false)
const error = ref('')
const open = ref(null)
const post = ref({})
const roles = computed(() => (props.guild?.roles || []).filter(r => r.name !== '@everyone' && !r.managed).map(r => ({ title: r.name, value: r.id })))
const textChannels = computed(() => (props.guild?.channels || []).filter(c => c.type === 0 || c.type === 5).map(c => ({ title: `#${c.name}`, value: c.id })))
const roleName = id => props.guild?.roles?.find(r => r.id === id)?.name || 'Role'
const dirty = computed(() => cfg.value && JSON.stringify(cfg.value) !== saved.value)
const styles = [{ title: 'Buttons', value: 'buttons' }, { title: 'Dropdown', value: 'dropdown' }]
const modes = [{ title: 'Pick any, click again to remove', value: 'toggle' }, { title: 'Only one at a time', value: 'unique' }, { title: 'Add only, no removing', value: 'add' }]

function load(c) {
  cfg.value = c
  saved.value = JSON.stringify(c)
}
onMounted(async () => {
  try { load((await api(`/api/guilds/${props.guildId}/modules/roles`)).config) } catch (e) { error.value = e.message }
})
function addMenu() {
  cfg.value.menus.push({ id: '', title: 'Pick your roles', description: '', color: '#66BB6A', style: 'buttons', mode: 'toggle', roles: [] })
  open.value = cfg.value.menus.length - 1
}
function setRoles(m, ids) {
  m.roles = ids.map(id => m.roles.find(r => r.role_id === id) || { role_id: id, label: '', emoji: '', description: '' })
}
async function save() {
  saving.value = true
  error.value = ''
  try {
    load((await api(`/api/guilds/${props.guildId}/modules/roles`, { method: 'PUT', body: JSON.stringify(cfg.value) })).config)
    emit('toast', 'Role menus saved')
  } catch (e) { error.value = e.message }
  saving.value = false
}
async function send(i) {
  error.value = ''
  try {
    if (dirty.value) await save()
    if (error.value) return
    await api(`/api/guilds/${props.guildId}/rolemenus/${cfg.value.menus[i].id}/send`, { method: 'POST', body: JSON.stringify({ channel_id: post.value[i] }) })
    emit('toast', 'Posted')
  } catch (e) { error.value = e.message }
}
</script>

<template>
  <div class="settings-form">
    <div v-if="!cfg && !error" class="row-list"><div v-for="i in 3" :key="i" class="skeleton-row"><span /></div></div>
    <template v-if="cfg">
      <p class="muted small" style="margin:0 0 20px">Members pick their own roles with buttons or a dropdown. Kev's role has to be above these roles.</p>
      <p v-if="!cfg.menus.length" class="muted small">No role menus yet.</p>
      <div v-for="(m, i) in cfg.menus" :key="i" class="menu">
        <button class="menu-head" @click="open = open === i ? null : i">
          <span>{{ m.title || 'Untitled' }}</span>
          <span class="muted small">{{ m.roles.length }} roles · {{ m.style }}</span>
        </button>
        <div v-if="open === i" class="menu-body">
          <div style="display:flex;gap:16px">
            <v-text-field v-model="m.title" label="Title" style="flex:3" />
            <v-text-field v-model="m.color" label="Color" style="flex:1" />
          </div>
          <v-textarea v-model="m.description" label="Text" rows="2" auto-grow class="field-block" />
          <div style="display:flex;gap:16px;flex-wrap:wrap">
            <v-select v-model="m.style" :items="styles" label="Style" style="flex:1;min-width:160px" />
            <v-select v-model="m.mode" :items="modes" label="Mode" style="flex:2;min-width:220px" />
          </div>
          <v-select :model-value="m.roles.map(r => r.role_id)" :items="roles" label="Roles" multiple chips closable-chips :loading="!guild" class="field-block" @update:model-value="ids => setRoles(m, ids)" />
          <div v-for="r in m.roles" :key="r.role_id" class="role-row">
            <v-text-field v-model="r.emoji" label="Emoji" hide-details density="comfortable" style="max-width:90px" />
            <v-text-field v-model="r.label" :placeholder="roleName(r.role_id)" persistent-placeholder label="Label" hide-details density="comfortable" />
            <v-text-field v-if="m.style === 'dropdown'" v-model="r.description" label="Description" hide-details density="comfortable" />
          </div>
          <div class="action-row">
            <v-select v-model="post[i]" :items="textChannels" label="Post in" hide-details density="comfortable" style="max-width:240px" />
            <v-btn variant="tonal" :disabled="!post[i] || !m.roles.length" @click="send(i)">Post</v-btn>
            <v-btn variant="text" @click="cfg.menus.splice(i, 1); open = null; post = {}">Delete menu</v-btn>
          </div>
        </div>
      </div>
      <v-btn variant="tonal" size="small" :disabled="cfg.menus.length >= 25" style="margin-top:12px" @click="addMenu">New role menu</v-btn>
      <div class="action-row">
        <v-btn color="primary" :loading="saving" :disabled="!dirty" @click="save">Save</v-btn>
        <span v-if="dirty" class="muted small">Unsaved changes</span>
      </div>
    </template>
    <p v-if="error" class="error-text small">{{ error }}</p>
  </div>
</template>

<style scoped>
.menu { border-top: 1px solid rgba(255, 255, 255, .06) }
.menu-head { all: unset; cursor: pointer; display: flex; justify-content: space-between; align-items: center; width: 100%; padding: 16px 0; font-weight: 600 }
.menu-body { padding: 0 0 16px }
.role-row { display: flex; gap: 10px; margin: 0 0 10px }
</style>
