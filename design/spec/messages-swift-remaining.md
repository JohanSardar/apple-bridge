# Messages decisions still in Swift

State after the Rust self-test launcher: the Messages send, targeted and conversation operations, the policy, the
route proofs, the correlation and the self-test step order are Rust (`session/messages.rs`, `messages_policy.rs`,
`messages_selftest.rs`). This lists what Swift still decides, so the removal PR can be written. Line numbers are
`native/Sources/MessagesSource` unless a path is given.

## 1. Old command path (duplicated in Rust; removable once Messages is Rust-routed by default)

Session mode hands Messages commands to Rust by default (`AB_SESSION_RUST_COMMANDS` in `AppleSession/SessionProxy.swift`).
Only `ipc/1` mode still serves them through `Messages.swift`, and the rows below go when #843 removes that mode.

| Swift | Decision | Rust owner |
|---|---|---|
| `MessagesCommands.prepare` 359, `prepareAccessibility` 383, `prepareConversation` 423, `latestConversationDate` 420 | origin, policy, route, tag and ledger rules, expected-state conflict | `messages.rs` `prepare`, `prepare_targeted`, `prepare_conversation` |
| `verifyRoute` 117, `validateResend` 138, `validateAgentPolicy` 143, `validateOwnSend` 96, `recipients` 87, `ownsRecipients` 93 | route, resend, agent policy, own recipients | `verify_route`, `validate_resend`, `validate_agent_policy`, `messages_policy::owns_recipients` |
| `validate` 495, `dispatchAccessibility` 499, `dispatchConversation` 462 | dispatch-time revalidation, UI effect observation, read-state matcher | `validate_send`, `write_targeted`, `write_conversation`, `messages_policy::read_state_matches` |
| `dispatch` 585, `parts` 559/562, `stagedAudioPart` 554, `correlate` 637, `aggregate` 663, `resolve` 668 | per-part send order, chat.db correlation, aggregate disposition, Resolve | `write_send`, `correlate`, `aggregate`, `resolved` |
| `MessagesSendPolicy.swift`, `MessageActionWindow.swift`, `MessagesRecipient.swift`, `MessagesReadStateFixture.swift` (`matches`, `authorizes`, `validate`) | tag rule, windows, recipient match, read-state matcher and ledger authorisation | `messages_policy.rs` |
| `MessagesSource/Messages.swift` `commandCoverage` and `MessagesCommands.capabilities`, `operationCoverage` | which Messages operations are advertised, from the route proofs | `messages_advertise.rs` |
| test targets `MessagesSafetyChecks` (`PolicyVectors.swift`, `main.swift`), `MessagesCaptureChecks/Rich.swift`, `Tests/CommandOriginChecks` | the Swift copies of the 43 policy vectors and origin rules | `tests/messages_policy_vectors.rs`, `messages.rs` tests |
| `SourceSupport/CommandOrigin.swift` | origin parsing; also used by `AppleSession`, `NativeSessionAdapter`, `HelperLabCommand` | `messages.rs` `origin_of` |

## 2. Decisions inside the Swift primitives

None left. `session/messages_local.rs` answers chat.db and file facts in Rust, `messages_policy` holds the identity,
self-chat, isolation, media and cleanup rules, `session/messages_ui.rs` selects Accessibility elements from what
`messages.ax_*` reports, and `session/messages_cleanup.rs` selects what a run may delete. Swift's `primitive` answers
only `config`, `me_card`, `send_part`, `delete_owned_item` and `ax_*`.

## 3. Primitive-only (stays in Swift)

`MessagesAXPrimitives`, `MessagesNavigation`, `MessagesUISession` input and fencing, `MessagesAppleScript`, the IMCore
move to Recently Deleted, the Contacts me card and the media facts.

## 4. Removed in the accompanying draft PR

`AppleSession/MessagesSelfTest.swift` (207 lines): the step list, the argument shapes, the failed-kind proof
invalidation, the cleanup-state computation and the report. Its callers moved to `RustSelfTest`. It is the only
mechanical removal today; every row of section 1 waits for the Helper default and `ipc/1` mode.
