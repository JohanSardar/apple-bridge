# Messages decisions still in Swift

None for commands. `session/messages.rs`, `messages_local.rs`, `messages_ui.rs`, `messages_cleanup.rs`,
`messages_selftest.rs`, `messages_advertise.rs` and `messages_policy.rs` own every rule: origin, policy, route, tags,
ledger, windows, correlation, the self chat, baselines, which row a cleanup may delete, which Accessibility element is
the right one, and which operations are advertised.

## What stays in Swift

`MessagesCommands.primitive` answers `config`, `me_card`, `send_part`, `delete_owned_item` and the `ax_*` primitives
(design/spec/native-1.md). `MessagesAXPrimitives`, `MessagesNavigation`, `MessagesUISession` input and fencing,
`MessagesAppleScript` and the IMCore move to Recently Deleted are the framework calls behind them.

## Not commands

The Swift Messages capture (`Messages.swift`, `MessagesReader.swift`, `MessageActionWindow.swift`, `MessagesPins.swift`,
`MessagesWatcher.swift`, `MessagesBaseline.swift`) remains for a home with no `session/rust-capture.json` and goes with the
capture cleanup. `MessagesMediaFact` stays: it is framework work.

## Not proven against real Messages

The Accessibility selection has run only against a fake tree and in part on the real Mac. The full self-test walk has
to run on the real account to prove it: `AppleBridgeHelper selftest messages --force --report <path>`.
