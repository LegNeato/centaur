# Zulipbot Guide

## Role

Zulipbot reads a Zulip event queue and applies the channel, direct-message,
guest, and bot-sender policy. It forwards each message addressed to the bot to
a durable Centaur session. It posts each final answer to the conversation of
the message. Zulipbot is a Rust service. The code is in
`crates/centaur-zulipbot`. This directory holds the Dockerfile and the docs.
`README.md` describes the behavior and the settings.

The key modules are these:

- `src/bot.rs` receives each message in the order of its conversation.
  `src/bot/hand_off.rs` starts or steers the execution.
  `src/bot/delivery.rs` posts the answer. `src/bot/scan.rs` finds the
  unfinished messages after a registration.
- `src/sync.rs` holds the conversation queues and the message claims.
- `src/listener.rs` reads the event queue and starts the recovery scan.
- `src/policy.rs` decides which messages start work.
- `src/status.rs` defines the status reactions.
- `src/turn.rs` builds the session API requests.
- `src/files.rs` downloads the uploads of a turn and builds their content
  blocks.
- `src/command.rs` recognizes the stop command.
- `src/flags.rs` parses the inline flags.
- `src/requester.rs` builds the requester identity and the GitHub
  attribution text.
- `crates/centaur-zulip` implements the Zulip API behind the `ZulipApi`
  trait. api-rs uses its client for the `zulip` connector.

## Invariants

- Run one replica. Each replica registers its own event queue. Two replicas
  therefore handle every message twice. Keep `replicas: 1` and the `Recreate`
  strategy.
- Every policy list fails closed. An empty allowlist allows nothing. A guest
  and another bot each need their own allowlist. The bot ignores a sender
  whose account it cannot read.
- The bot keeps no durable state. api-rs deduplicates messages by Zulip
  message ID and executions by triggering message ID. The status reactions
  record delivery. A change that adds state to the bot must keep the restart
  behavior in `README.md` correct.
- Add a final status reaction before you remove the working reaction. The
  recovery scan resumes a message whose only status reaction is the working
  reaction.
- Read the uploads of a message from its rendered links. Do not search the
  Markdown source for upload paths.
- The stop phrases and the inline flags follow slackbotv2. Change them in
  both bots.
- `ZulipDestination` in `centaur-session-core` owns the thread-key format. Do
  not format or parse `zulip:` keys anywhere else.
- Never log the Zulip API key or the api-rs key. Never post api-rs error
  bodies to Zulip. Notices in Zulip stay generic.

## Validation

Run these commands from the repository root:

```bash
cargo test -p centaur-zulipbot
cargo clippy -p centaur-zulipbot --all-targets -- -D warnings
```

The tests use in-memory fakes of Zulip and the session API. Add coverage next
to the behavior that you change. A change at the Zulip boundary also needs a
run against a local Zulip server. The run must show that one mention produces
one execution, one answer, and one final reaction.
