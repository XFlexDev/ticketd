<script setup>
import { computed } from 'vue'
import { useDisplay } from 'vuetify'

const { width } = useDisplay()
const units = {
  wide: { key: '6626e80a85a19a6154791f608409a661', w: 728, h: 90 },
  small: { key: '689d4a1423c6583fe1ec2a1aa14a2ee3', w: 320, h: 50 },
}
const unit = computed(() => (width.value >= 780 ? units.wide : units.small))
// Opaque-origin sandbox: the ad can run and open its own tabs, but can't touch the dashboard or its session.
const doc = computed(() => `<!doctype html><html><head><style>html,body{margin:0;background:transparent;overflow:hidden}</style></head><body><script>atOptions={key:'${unit.value.key}',format:'iframe',height:${unit.value.h},width:${unit.value.w},params:{}};<\/script><script src="https://bauval.org/22/${unit.value.key}"><\/script></body></html>`)
</script>

<template>
  <div class="ad-slot" :style="{ height: unit.h + 'px' }">
    <iframe :key="unit.key" :srcdoc="doc" :width="unit.w" :height="unit.h" title="Advertisement" loading="lazy" scrolling="no" referrerpolicy="no-referrer-when-downgrade" sandbox="allow-scripts allow-popups allow-popups-to-escape-sandbox" />
  </div>
</template>
