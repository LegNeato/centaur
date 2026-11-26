# Zulipbot

`centaur-zulipbot` connects a Zulip organization to Centaur. The bot answers
@-mentions in allowed channels and direct messages from allowed users. Each
topic and each direct-message conversation is one Centaur session. The code is
in [`crates/centaur-zulipbot`](../../crates/centaur-zulipbot/).

## Behavior

The bot reads new messages from a Zulip event queue. It makes only outbound
requests and needs no public URL.

A message starts work in three cases:

- The message @-mentions the bot in a topic of an allowed channel.
- An allowed user sends the bot a one-to-one direct message.
- An allowed user @-mentions the bot in a group direct message.

A wildcard mention like `@**all**` does not start work. Two kinds of senders
need their own allowlist:

- A guest needs `ZULIPBOT_GUEST_ALLOWLIST` in channels and in direct messages.
  `ZULIPBOT_DIRECT_MESSAGE_ALLOWLIST=*` does not include guests.
- Another bot needs `ZULIPBOT_TRIGGER_BOT_ALLOWLIST`. Two bots therefore
  cannot answer each other in a loop.

The bot reads the account type of each sender from Zulip and keeps it for
five minutes. The bot ignores a message when the lookup fails.

The bot does these steps for each message that starts work:

1. It subscribes to the channel of the message when it is not subscribed. A
   direct message skips this step.
2. It reacts with 👀 (`eyes`).
3. It reads the earlier messages of the conversation that came after the last
   message it handled. `ZULIPBOT_CONTEXT_MESSAGES` limits the number.
4. It downloads the files that these messages and the request link to.
5. It creates or reuses the session. Then it appends the earlier messages and
   the request with their files.
6. It starts an execution. When an execution of the session is already
   running, the request steers that execution instead. The bot then reacts
   with ⏳ (`time_ticking`). The agent gets the steering message with the
   name of its sender. The flags of a steering message do not apply. The
   bot says so.
7. It renews a typing notification until the execution ends. It posts each
   progress update of api-rs in the conversation. Then it posts the final
   answer and replaces 👀 with ✅ (`check`) or ❌ (`cross_mark`).

The bot handles the messages of one conversation in the order of the event
queue. A message waits until the message before it has started or steered
an execution. Messages of other conversations do not wait. The bot never
handles a message twice at the same time.

A message that steered an execution gets the ✅ or ❌ of the answer of that
execution. The same is true for a steer just after the end of the execution.
The bot keeps this record in memory. A restart between the steer and the
answer leaves the message with ⏳. The bot splits an answer at line ends when the answer is longer
than the message limit of the server. A code block that crosses a split is
closed at the end of one message and opened again at the start of the next.

api-rs writes progress updates when the deployment enables
`apiRs.activitySummary.progressUpdates`. A resumed delivery replays the
updates of its execution. The bot skips an update that is older than two
minutes.

## Commands and flags

A request that consists of a stop phrase interrupts the running execution
of the conversation. Examples are `@**Centaur** stop` and
`@**Centaur** please cancel the run`. The bot marks the stop request with ✅.
The interrupted request gets 🛑 (`stop_sign`) and no answer. A stop without a
running execution changes nothing. The recovery scan does not replay a stop.

A request can hold the inline flags of slackbotv2. The bot removes them from
the text that the agent reads.

| Flag | Effect |
| --- | --- |
| `--codex`, `--claude`, `--claude-code`, `--amp`, `--hermes`, `--nanocode`, `--pi`, `--omp` | Selects the harness. A session on another harness restarts on the new harness. The new harness gets the earlier messages of the conversation and the earlier answers. |
| `--model <name>` | Selects the model of the turn. `fable`, `opus`, `sonnet`, and `haiku` expand to the current Claude model IDs. |
| `--fable`, `--opus`, `--sonnet`, `--haiku` | Selects that Claude model and Claude Code. |
| `--provider <name>`, `--bedrock`, `--meta` | Selects the model provider of the turn and Codex. |
| `-rsn <effort>` | Selects the reasoning effort of the turn, for example `high`. |
| `--persona <id>` | Selects the persona of a new session. |

A harness flag for a harness outside `ZULIPBOT_ENABLED_HARNESSES` has no
effect. The bot then ignores the model and provider flags too and says so.
A value flag also accepts `=`, for example `--model=opus`. A session keeps its
harness until a harness flag changes it. A model applies to its turn and stays
while the harness process runs. Repeat the flag after an idle pause of the
sandbox. A persona that pins a model overrides `--model`. The bot says
so in the conversation. The bot also says so when a requested persona is not
available.

A model or provider of the custom provider catalog runs on Codex with its
provider. The chart renders `codex.customProviders` as `CODEX_CUSTOM_PROVIDERS`
for the bot. A catalog model matches its ID, `<provider>/<id>`, or an alias. A
provider without `--model` gets the `defaultModel` of the provider. The bot
rejects a catalog model that names models of two providers. It also rejects a
catalog model with a harness flag other than `--codex`. The request then gets
❌ and a notice. A new session that the rejected request opens still gets the
`--persona` of the request.

`ZULIPBOT_CHANNEL_DEFAULTS` sets the defaults of a channel as a JSON object
keyed by channel ID. Each value can name a `persona`, a `harness`, a `model`, a
`provider`, and a `reasoning` effort in the vocabulary of the flags. A request
that selects a harness, a model, or a provider replaces the harness, model,
and provider of the default. The default persona and effort still apply. The
default harness and the default persona apply only to a new session. The
default model and provider apply only when the session runs the default
harness. The bot does not start when a default selects a harness outside
`ZULIPBOT_ENABLED_HARNESSES` that is not `ZULIPBOT_HARNESS`.

```json
{"42": {"persona": "eng", "harness": "claude", "model": "opus", "reasoning": "high"}}
```

An answer can end with a line that names the model, the custom provider, the
harness, and the reasoning effort, for example `Opus 5.5 · Claude Code · High`.
`ZULIPBOT_RESPONSE_METADATA_MODE` selects `first`, `always`, or `never`.
`first` shows the line on the first answer of a conversation and on an answer
to a request with a harness, model, or provider flag. The bot reads the
default models from the harness config that the sandbox image bakes in.
`CLAUDE_MODEL`, `CODEX_MODEL`, `CODEX_MODEL_REASONING_EFFORT`,
`CENTAUR_PI_MODEL`, and `CENTAUR_OMP_MODEL` replace those defaults. The chart
copies them from `sandbox.extraEnv`.

## Agent posts

The `zulip` connector lets the agent post to its conversation.
`zulip.upload_file` uploads a sandbox file and posts a message that links it.
`zulip.post_message` posts a Markdown message. The agent's final answer still
comes from the bot. The connector calls `POST /api/zulip/files` and
`POST /api/zulip/messages` on api-rs with the sandbox token. api-rs finds the
session that holds the sandbox and posts to its Zulip conversation as the bot.
A request cannot name another conversation. api-rs streams a file of up to
100 MiB to Zulip. The Zulip server can set a lower limit. The chart gives
api-rs the bot credentials and the egress to `zulipbot.zulipPorts` when
`zulipbot.enabled` is true.

## Requester identity

The agent gets the Zulip ID, the name, and the mention syntax of the sender.
It also gets the GitHub handle from the Zulip profile of the sender and a
rule that credits the sender in a GitHub pull request. The bot reads the
handle from a **GitHub** external account field first. Then it reads any
custom profile field whose name mentions GitHub. A value can be a login, a
profile URL, or a text such as `GitHub: octocat`.

## Channels

`ZULIPBOT_CHANNEL_ALLOWLIST` selects the channels where the bot answers. It
holds channel IDs, or `*` for every channel.

Zulip delivers a direct @-mention of a generic bot in any channel, also when
the bot is not subscribed to the channel. The bot subscribes to each allowed
channel where an allowed user mentions it. A user group mention reaches the
bot only in a channel that it is subscribed to.

Zulip lets the bot subscribe to a private channel only with a grant. A member
can subscribe the bot to the channel. An administrator can also add the bot to
the **Who can subscribe** setting of the channel. Without a subscription the
bot cannot read the history, the files, or the reactions of a private channel.
A mention there therefore starts no work. The bot sends the sender a direct
message that asks for a subscription. The recovery scan does not send this
notice again.

## Files

A Zulip message links each upload with a `/user_uploads/` URL. The Zulip API
has no list of the files of a message. The server finds them when it renders
the message. The bot reads the links of the rendered message in the same way.
It reads them only for a message whose text contains `user_uploads/`. Text in
code and a bare path are not links.

The bot downloads the files of the request and of the earlier messages with
its own credentials. The request comes first. The earlier messages follow from
the newest to the oldest. One turn attaches at most 20 files and at most
`ZULIPBOT_TURN_FILE_BYTES` bytes. The bot skips a file that does not fit. A
later and smaller file can still fit.

Each file goes to the agent as a harness attachment block. The harness saves
the file in the sandbox and shows an image to the model. A text block before
each file names its upload link. The agent can therefore match a file to the
link in a message. A text block explains each skipped file.

api-rs stores the bytes in the session message and in the execution request.
A steered message therefore delivers its files to the running execution.

> **Note:** The bot downloads with the permissions of its own account. A user
> who knows the upload link of a file that the bot can read can make the agent
> read it. An upload link holds a random part. Only the readers of the file see
> it.

## Admission

api-rs can admit only the conversations that an operator approved. The
setting is `CENTAUR_SESSION_PRINCIPAL_ADMISSION=preapproved`. The approval
applies to the principal of the conversation. That principal is a channel, the
user of a one-to-one direct message, or the participants of a group direct
message. The bot removes its reaction and posts nothing in a conversation that
is not approved. Admission does not check the sender. The guest and bot
allowlists therefore still apply in an approved channel.

## Thread keys

The control plane gets one thread key for each conversation:

- The key of a topic is `zulip:channel:<channel_id>:<topic>`. The topic is
  lowercase because Zulip compares topic names without regard to case.
- The key of a direct-message conversation is
  `zulip:dm:<user_id>,<user_id>,...`. It lists the participants other than the
  bot in ascending order.

`ZulipDestination` in `centaur-session-core` defines both formats. The bot
mints keys with it, and api-rs parses keys with it. The agent gets a context
line that names the conversation. `session_context.zulip` holds the channel ID
and the topic, or the participant IDs. Tools read it with
`current_zulip_thread()` from the tool SDK.

iron-control gives every topic of a channel the principal of the channel. A
one-to-one direct message gets the principal of the other user. A group direct
message gets a principal for its set of participants.

## Restarts and delivery

The bot keeps no state of its own:

- api-rs stores each appended message once, keyed by its Zulip message ID.
- api-rs starts one execution for each triggering message ID.
- The reactions of the bot record which messages it handled.

The bot scans its mentions and its direct messages after each queue
registration. The scan goes back to the start of the recovery window,
`ZULIPBOT_RECOVERY_WINDOW_SECONDS`. It handles every message in the window
that has no final reaction:

- A message with 👀 is resumed. The bot rejoins its execution and posts the
  answer.
- A message without a reaction from the bot is handled as a new message.

A failed scan runs again up to four times within about two minutes.

Set the recovery window to at least the longest execution. A shorter window
leaves the answers of long executions undelivered after a restart. A restart
after the bot posts an answer and before it adds ✅ makes the bot post the
answer again.

The bot keeps waiting for an execution while api-rs is unavailable. It waits
30 seconds after the session client uses its reconnects. Then it follows the
execution again. It posts only the progress updates that are newer than the
ones it posted. An execution that api-rs no longer knows gets ❌ and a
notice.

> **Run exactly one replica.** Each replica registers its own event queue and
> handles every message. Two replicas therefore answer every message twice. The
> chart deploys one replica with the `Recreate` strategy.

## Zulip setup

The bot needs Zulip Server 9.0 or later, or Zulip Cloud. Zulip 9.0 has API
feature level 250.

1. Create the bot in **Organization settings → Bots → Add a new bot**.
   **Personal settings → Bots** opens the same form. Select the
   **Generic bot** type. Note its email address and API key.

   The user who creates the bot owns it. Only the owner can see or replace
   its API key. Zulip deactivates the bots of a deactivated user. The bots
   stay deactivated after Zulip reactivates the user. Give the bot an owner
   that stays active. An administrator or a shared service account can be
   that owner. Only an administrator can change the owner of a bot. The
   administrator does this in **Organization settings → Bots**.
2. Set `ZULIPBOT_CHANNEL_ALLOWLIST` to the channel IDs or to `*`. The URL of a
   channel holds its ID, for example `#narrow/channel/42-engineering`.
   Subscribe the bot to each private channel where it answers, or let it
   subscribe through the **Who can subscribe** setting of the channel.
3. Set `ZULIPBOT_DIRECT_MESSAGE_ALLOWLIST` to user IDs or to `*` to allow
   direct messages. The profile of a user shows the user ID.

The bot ignores every message until one of the allowlists is set.

## Configuration

Every setting is also a command-line flag. `centaur-zulipbot --help` lists the
flags.

| Variable | Default | Description |
| --- | --- | --- |
| `ZULIP_SITE` | required | The organization URL, for example `https://chat.example.com`. |
| `ZULIP_EMAIL` | required | The email address of the bot. |
| `ZULIP_API_KEY` | required | The API key of the bot. |
| `CENTAUR_API_URL` | `http://127.0.0.1:8080` | The base URL of api-rs. |
| `ZULIPBOT_API_KEY` | required | The ingress key that api-rs accepts for `zulip:` sessions. Use a dedicated key. |
| `ZULIPBOT_CHANNEL_ALLOWLIST` | empty | The channel IDs where the bot answers, or `*` for every channel. Commas or spaces separate the IDs. |
| `ZULIPBOT_DIRECT_MESSAGE_ALLOWLIST` | empty | The user IDs that may send the bot direct messages, or `*` for every member. |
| `ZULIPBOT_TRIGGER_BOT_ALLOWLIST` | empty | The bot account IDs whose messages may start work. |
| `ZULIPBOT_GUEST_ALLOWLIST` | empty | The guest account IDs whose messages may start work, or `*` for every guest. |
| `ZULIPBOT_HARNESS` | `codex` | The harness that new sessions run. |
| `ZULIPBOT_ENABLED_HARNESSES` | `*` | The harnesses that a harness flag can select, for example `codex,claudecode`. |
| `ZULIPBOT_CHANNEL_DEFAULTS` | empty | The persona, harness, model, provider, and reasoning defaults of each channel as JSON. |
| `CODEX_CUSTOM_PROVIDERS` | empty | The custom Codex providers and their models as JSON. |
| `ZULIPBOT_RESPONSE_METADATA_MODE` | `first` | When an answer gets the model and harness line: `first`, `always`, or `never`. |
| `CLAUDE_MODEL`, `CODEX_MODEL`, `CODEX_MODEL_REASONING_EFFORT`, `CENTAUR_PI_MODEL`, `CENTAUR_OMP_MODEL` | baked harness config | The sandbox defaults that the model and harness line names. |
| `ZULIPBOT_CONTEXT_MESSAGES` | `50` | The maximum number of earlier messages that one turn reads, from 1 to 1000. |
| `ZULIPBOT_RECOVERY_WINDOW_SECONDS` | `3600` | The age limit of the messages that the recovery scan handles. |
| `ZULIPBOT_TURN_FILE_BYTES` | `26214400` (25 MiB) | The maximum total size of the files that one turn attaches. `0` attaches no files. |
| `SESSION_IDLE_TIMEOUT_MS`, `SESSION_MAX_DURATION_MS` | api-rs default | The execution limits that the bot sends to api-rs. |
| `BIND_ADDR` | `0.0.0.0:8080` | The address of the health endpoint. |
| `RUST_LOG` | `info` | The log filter. |

`GET /healthz` returns 200 while the event queue answers. It returns 503
before the first queue registration. It also returns 503 after five minutes
without a successful poll.

## Limitations

- The answer appears when the execution ends. The typing notification and
  the progress updates show the work in progress.
- A renamed topic starts a new session. Messages moved to another topic also
  continue in the session of that topic.
- The bot does not answer messages older than the recovery window.

## Develop

Run these commands from the repository root:

```bash
cargo test -p centaur-zulipbot
cargo clippy -p centaur-zulipbot --all-targets -- -D warnings
```

The tests replace Zulip and the session API with in-memory fakes. They need no
Zulip server.
