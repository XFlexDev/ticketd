<script setup>
import { computed } from 'vue'
import { useRoute } from 'vue-router'

const route = useRoute()
const doc = computed(() => (route.path.startsWith('/privacy') ? privacy : terms))
const updated = 'October 20, 2026'

const terms = {
  title: 'Terms of Service',
  sections: [
    ['Who we are', ['Kev is a free Discord bot and dashboard at kev.kreatix.dev, made by Kreatix, a personal project. Questions go to info@kreatix.dev.', 'By adding Kev to a server, using its commands or signing in to the dashboard, you agree to these terms and to the Privacy Policy.']],
    ['Discord rules come first', ['You must follow the Discord Terms of Service and Community Guidelines while using Kev. You must be old enough to use Discord where you live.']],
    ['Server admins', ['Admins decide which modules are on and how they behave, including AI instructions, knowledge files, webhooks, MCP servers, custom commands, auto replies, alerts and automod rules. Admins are responsible for that setup and for what it sends to their members and to outside services.', 'Moderation and automod actions are taken on behalf of the server. Appeals go to that server\'s staff, not to us.']],
    ['Acceptable use', ['Do not use Kev to spam, harass, scam or threaten anyone, to share illegal content, to break into systems, or to get around Discord or Kev limits.', 'Only upload files and play audio you have the right to share. Do not point webhooks, MCP servers or alerts at services you are not allowed to use.', 'Do not try to overload, reverse engineer or attack Kev or the dashboard.']],
    ['AI replies', ['Ticket replies can be written by AI. They can be wrong or incomplete and are not professional, legal, medical or financial advice. Staff should check anything important.']],
    ['Free service', ['Kev is free and supported by ads on the dashboard. There are no ads inside Discord.', 'Kev is provided as is, with no guarantee that it is always online, error free or that data is never lost. We can change, limit or remove features at any time.']],
    ['Ending access', ['We can block any server or user that breaks these terms or puts Kev or its users at risk. Admins can remove Kev at any time by kicking it from their server.']],
    ['Liability', ['As far as the law allows, we are not liable for indirect damage, lost data, lost profits or anything caused by how a server sets up or uses Kev. Nothing here limits rights you have under mandatory consumer law.']],
    ['Changes and law', ['We may update these terms. The date at the top shows the latest version, and using Kev after a change means you accept it. Finnish law applies.']],
  ],
}

const privacy = {
  title: 'Privacy Policy',
  sections: [
    ['Who is responsible', ['Kev is run by Kreatix, a personal project. For anything about your data, email info@kreatix.dev.']],
    ['What Kev stores', [
      'Discord basics: user, server, channel and role IDs, usernames and avatars, as needed to run the features below.',
      'Server settings that admins save in the dashboard, plus up to 50 earlier versions as backups.',
      'Tickets: messages, attachments, form answers, ratings, staff notes and transcripts. Attachments and transcripts are copied to file storage so they keep working after Discord links expire.',
      'Moderation: cases with the moderator, member, action, reason and time.',
      'Feature data: XP and message counts for leveling, giveaway entries, reminders, AFK status, suggestion votes, sticky messages, starboard entries and music lobby owners.',
      'Dashboard sign in: your Discord account and server list through Discord login, and a session cookie that keeps you signed in.',
    ]],
    ['What Kev only looks at', [
      'Automod, auto replies and leveling read messages to apply their rules. That text is not saved.',
      'For logging, Kev keeps the last 200 messages per channel in memory so it can show what an edited or deleted message said. This is wiped when Kev restarts.',
      'Songs and stations played in voice are not saved after the session.',
    ]],
    ['How it is used', ['Only to run Kev for the servers that use it. We do not sell your data, share it for advertising or build profiles about you.']],
    ['AI', ['In tickets where the server turned AI on, the ticket conversation, form answers and the server\'s knowledge files are sent to an AI inference provider to write a reply. They are used only to produce that reply.']],
    ['Services we use', [
      'Discord, to run the bot and sign you in.',
      'Cloudflare, for the website, image delivery and the verification CAPTCHA (Turnstile), which checks your browser to tell people from bots.',
      'Kreatix Storage on Backblaze B2 in the EU, for attachments, transcripts, uploaded images, knowledge files, backups and exports.',
      'An AI inference provider, for ticket replies.',
      'Adsterra, for the ads on the dashboard. Ads run on ads.kreatix.dev, a separate site that cannot read your dashboard session, and Adsterra may use its own cookies there.',
    ]],
    ['How long it is kept', [
      'Server data stays while Kev is in the server or until an admin deletes it, or until you ask us to delete it.',
      'Exports are deleted after 7 days, uploads that were never saved after 24 hours, and backups beyond the latest 50.',
      'Reminders are deleted once sent and AFK status once you talk again.',
    ]],
    ['Your rights', ['Under the GDPR you can ask to see, correct, export or delete your data, or object to how it is used. Email info@kreatix.dev and we answer within 30 days. Server admins can also export their server\'s data from the dashboard.', 'You can also complain to the Finnish Data Protection Ombudsman (tietosuoja.fi).']],
    ['Legal basis', ['We process data to provide the service you or your server asked for, and for our legitimate interest in keeping Kev safe and working.']],
    ['Changes', ['We may update this policy. The date at the top shows the latest version.']],
  ],
}
</script>

<template>
  <div class="legal">
    <router-link to="/" class="muted small back">Kev</router-link>
    <h1>{{ doc.title }}</h1>
    <p class="muted small">Last updated {{ updated }}</p>
    <section v-for="[heading, paras] in doc.sections" :key="heading">
      <h2>{{ heading }}</h2>
      <p v-for="(p, i) in paras" :key="i">{{ p }}</p>
    </section>
    <p class="muted small swap">
      <router-link v-if="doc === terms" to="/privacy">Privacy Policy</router-link>
      <router-link v-else to="/terms">Terms of Service</router-link>
    </p>
  </div>
</template>

<style scoped>
.legal { max-width: 720px; margin: 0 auto; padding: 40px 24px 80px }
.legal h1 { margin: 12px 0 4px }
.legal h2 { margin: 32px 0 8px; font-size: 18px }
.legal p { margin: 0 0 10px }
.back { text-decoration: none }
.swap { margin-top: 40px }
</style>
