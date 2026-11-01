# xenify-chat-server

The chat part of the Xenify server: direct chats, groups and channels. Accounts
and signing in are the Xenify server's.

- **Direct chats:** one per pair of people; a chat with yourself is "Notes to self".
- **Groups:** everyone talks, and anyone in a group can add people. Anyone can leave. When the owner leaves, whoever has been in longest takes over; the last one out deletes the group.
- **Channels:** owners (and apps) post, others follow. Open channels can be found and subscribed to; closed ones only by invitation.
- **Read markers, notification settings and each person's Chat settings** are kept per person and follow them to all their devices.
- **Search:** the messages someone may read (their chats, and channels they follow), by word or the start of one, best first with the matching part marked (PostgreSQL text search).
- **Keeping messages:** for ever, or as long as an admin chooses (Admin › Network › Chat); owners of a group or channel can choose less. About once an hour older messages go, with the files sent in those chats then. Calendars, meetings and address books kept in conversations stay.
- **Reminders:** the apps send each person's reminders (from their calendar and "Remind me…"); when one is due every device of theirs shows it, and snoozing or dismissing it on one clears it on the others. The Xenify server also sends it to their browsers (Web Push), with Snooze and Dismiss buttons that work with the page closed.
- **Missed messages by email:** direct messages and mentions (or everything) still unread after ten minutes, following mutes, Do not disturb and quiet hours, when the Xenify server can send email.

## Run it

It runs inside `xenify-server`, on the same PostgreSQL database. Run the server
on several machines with the same database to share the load. Message order,
and delivery to people connected to other servers, go through PostgreSQL.

## Apps: bots, alerts and feeds

People make apps themselves: in Xenify Chat, Settings › Apps › *Make an app*, or
from a script with their own API key (Settings › Apps › *Make an API key*).
Making one gives its key and webhook secret, shown once. Only its maker can
change it, give it a new key or delete it. Apps can't make apps.

Who an app talks with:
- **Each person privately**: everyone gets their own 1:1 chat with it, and
  nobody sees anyone else's. In groups and channels it's in, people see *Ask
  privately*.
- **In groups and channels**: it posts where it's added and hears messages
  that @mention it. It can't be messaged 1:1.
- **Both**.

### The API (`Authorization: Bearer <key>`)

| | |
|---|---|
| `POST /v1/post {"conv": 12 \| "channel": "Build alerts", "text": "…"}` | Post as the app → `{"conv", "seq", "id"}` |
| `POST /v1/edit {"conv", "seq", "text"}` | Change the app's post; send it again as it grows to stream an answer |
| `GET /v1/apps` · `POST /v1/apps {"name", "display_name", "description", "webhook", "mode"}` | Your apps · make one (with your API key) |
| `POST /v1/apps/update` · `/v1/apps/key` · `/v1/apps/delete` | Change (`{"app": id, …}`), new key and secret, delete |

### Webhooks (two-way, no connection to keep open)

Messages for an app (in its 1:1 chats if it's personal; @mentions in groups
and channels if it's shared) are POSTed to its webhook as JSON:

```json
{"event": "message", "app": "helper", "conv": 12, "kind": "direct", "message": {…}}
```

with `X-Xenify-Timestamp` and `X-Xenify-Signature: sha256=<hex>`, the
HMAC-SHA256 of `"<timestamp>.<body>"` with the app's secret. Check it, and
ignore old timestamps. Answer `200` with `{"text": "…"}` to reply at once, or
reply later with `/v1/post`. Messages from apps aren't sent on, so bots can't
loop.

Webhooks must be public `https://` addresses: addresses on this computer or a
private network are refused (and checked again before each delivery), so the
server can't be used to reach machines behind it. For testing on one machine,
start `xenify-server` with `--allow-local-webhooks`.

## Licence

MIT.
