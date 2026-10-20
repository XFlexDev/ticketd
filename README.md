# ticketd

Free, self-hostable Discord ticket bot in Rust with an AI support agent in the loop.

- Slash commands + button panels
- Private ticket channels under a category you pick
- AI auto-answers tickets (OpenAI-compatible endpoint — NVIDIA NIM free tier works great), rotates multiple API keys
- AI summary + transcript file when a ticket closes
- Per-server config stored in KreatixDB — one bot serves many servers

## Commands

| Command | Who | What |
|---|---|---|
| `/setup` | Manage Server | pick ticket category, mod role, optional log channel, toggle AI |
| `/panel` | mods | post an "Open a ticket" button panel |
| `/open` | anyone | open a ticket with an optional subject |
| `/close` | anyone in ticket | close, summarize, save transcript |
| `/add`, `/remove` | anyone in ticket | share a ticket with another user |
| `/transcript` | anyone in ticket | download the message log |

## Run

```sh
cp .env.example .env   # fill in
cargo run --release
```

Bot needs the `bot` + `applications.commands` scopes and Manage Channels + Send Messages permissions. Enable the Message Content intent in the dev portal.
