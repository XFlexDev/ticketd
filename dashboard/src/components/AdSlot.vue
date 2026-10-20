<script setup>
import { computed } from 'vue'
// Ad pages run sandboxed without allow-same-origin: they get kev.kreatix.dev as referrer but can't touch the dashboard or its session.
import { useDisplay } from 'vuetify'

const { width } = useDisplay()
const units = {
  wide: { key: '6626e80a85a19a6154791f608409a661', w: 728, h: 90 },
  small: { key: '689d4a1423c6583fe1ec2a1aa14a2ee3', w: 320, h: 50 },
}
const unit = computed(() => (width.value >= 780 ? units.wide : units.small))
</script>

<template>
  <div class="ad-slot" :style="{ height: unit.h + 'px' }">
    <iframe :key="unit.key" :src="`/ad/${unit.w}.html`" :width="unit.w" :height="unit.h" title="Advertisement" loading="lazy" scrolling="no" referrerpolicy="no-referrer-when-downgrade" sandbox="allow-scripts allow-popups allow-popups-to-escape-sandbox" />
  </div>
</template>
