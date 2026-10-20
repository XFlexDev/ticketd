<script setup>
import { ref } from 'vue'
import { mdiUpload, mdiClose } from '@mdi/js'

const props = defineProps({ modelValue: { type: String, default: '' }, label: { type: String, required: true }, guildId: { type: String, required: true } })
const emit = defineEmits(['update:modelValue', 'error'])
const input = ref(null)
const busy = ref(false)

async function pick(e) {
  const f = e.target.files?.[0]
  e.target.value = ''
  if (!f) return
  if (f.size > 8 * 1024 * 1024) return emit('error', 'Images can be up to 8 MB')
  busy.value = true
  try {
    const r = await fetch(`/api/guilds/${props.guildId}/media`, { method: 'POST', credentials: 'same-origin', body: f })
    if (!r.ok) throw new Error((await r.text()) || r.statusText)
    emit('update:modelValue', (await r.json()).url)
  } catch (err) { emit('error', err.message) }
  busy.value = false
}
</script>

<template>
  <div class="image-field">
    <div v-if="modelValue" class="img-thumb"><img :src="modelValue" alt=""></div>
    <v-text-field :model-value="modelValue" :label="label" placeholder="Upload or paste https://..." hide-details density="comfortable" @update:model-value="v => emit('update:modelValue', v)">
      <template #append-inner>
        <v-btn v-if="modelValue" variant="text" size="small" :icon="mdiClose" aria-label="Remove" @click="emit('update:modelValue', '')" />
        <v-btn variant="text" size="small" :icon="mdiUpload" :loading="busy" aria-label="Upload" @click="input.click()" />
      </template>
    </v-text-field>
    <input ref="input" type="file" accept="image/png,image/jpeg,image/gif,image/webp" hidden @change="pick">
  </div>
</template>
