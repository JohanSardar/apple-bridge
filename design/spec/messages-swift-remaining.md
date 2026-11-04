# Messages decisions still in Swift

State after the Rust self-test launcher: the Messages send, targeted and conversation operations, the policy, the
route proofs, the correlation and the self-test step order are Rust (`session/messages.rs`, `messages_policy.rs`,
`messages_selftest.rs`). This lists what Swift still decides, so the removal PR can be written. Line numbers are
`native/Sources/MessagesSource` unless a path is given.

## 1. Old command path (duplicated in Rust; removable once Messages is Rust-routed by default)

Not mechanical today. The Helper default (`AB_SESSION_RUST_COMMANDS` in `AppleSession/SessionProxy.swift:42`) does
not include `messages`, native session mode is opt-in (`AB_SESSION_NATIVE=1`), and `ipc/1` mode still serves
Messages Prepare/Validate/Dispatch/Resolve through `Messages.swift` (`prepare` 752, `dispatch` 760, `resolve` 761,
`validate` 796) called by `AppleSession.swift` (1084 validate, 1098 dispatch, 1127 capture) and
`NativeSessionAdapter.swift:243`. The `Source` protocol requires those four methods.

| Swift | Decision | Rust owner |
|---|---|---|
| `MessagesCommands.prepare` 359, `prepareAccessibility` 383, `prepareConversation` 423, `latestConversationDate` 420 | origin, policy, route, tag and ledger rules, expected-state conflict | `messages.rs` `prepare`, `prepare_targeted`, `prepare_conversation` |
| `verifyRoute` 117, `validateResend` 138, `validateAgentPolicy` 143, `validateOwnSend` 96, `recipients` 87, `ownsRecipients` 93 | route, resend, agent policy, own recipients | `verify_route`, `validate_resend`, `validate_agent_policy`, `messages_policy::owns_recipients` |
| `validate` 495, `dispatchAccessibility` 499, `dispatchConversation` 462 | dispatch-time revalidation, UI effect observation, read-state matcher | `validate_send`, `write_targeted`, `write_conversation`, `messages_policy::read_state_matches` |
| `dispatch` 585, `parts` 559/562, `stagedAudioPart` 554, `correlate` 637, `aggregate` 663, `resolve` 668 | per-part send order, chat.db correlation, aggregate disposition, Resolve | `write_send`, `correlate`, `aggregate`, `resolved` |
| `MessagesSendPolicy.swift`, `MessageActionWindow.swift`, `MessagesRecipient.swift`, `MessagesReadStateFixture.swift` (`matches`, `authorizes`, `validate`) | tag rule, windows, recipient match, read-state matcher and ledger authorisation | `messages_policy.rs` |
| `MessagesSource/Messages.swift` `commandCoverage` 764 and `MessagesCommands.capabilities` 49, `operationCoverage` 53 | which Messages operations are advertised, from the route proofs | still Swift: Rust reads `messages.route_proofs` but the advertised capability list is computed here |
| test targets `MessagesSafetyChecks` (`PolicyVectors.swift`, `main.swift`), `MessagesCaptureChecks/Rich.swift`, `Tests/CommandOriginChecks` | the Swift copies of the 43 policy vectors and origin rules | `tests/messages_policy_vectors.rs`, `messages.rs` tests |
| `SourceSupport/CommandOrigin.swift` | origin parsing; also used by `AppleSession`, `NativeSessionAdapter`, `HelperLabCommand` | `messages.rs` `origin_of` |

## 2. Decisions inside the Swift primitives that Rust still waits for

These run in Swift today because the primitive returns a verdict, not facts. Each could return the facts and let
`messages_selftest.rs` or `messages.rs` decide.

| Swift | Decision | Primitive |
|---|---|---|
| `MessagesSelfTestIsolation.target` 74, `accountHandles` 63, `isSelfChat` 42, identity normalisation 10-46 | which chat is the isolated self chat and which handle is the account's | `selftest_facts`, `is_self_chat` |
| `MessagesSelfTestIsolation.qualifiesFixtureChat` 55, `ownsMessage` 51, `ownsDeletedSelection` 59, `matchesConversation` 47 | whether the chat a send created may be touched, whether a message is the run's own | `selftest_fixture_chat`, cleanup, UI validate |
| `selftest_facts` `ui_allowed` (`MessagesCommands.swift`) | Accessibility trusted and screen unlocked, combined with `include_ui` | `selftest_facts` |
| `MessagesContentBaseline.capture` and `MessagesSelfTestBaselineFailure` (`MessagesSelfTestBaseline.swift` 21-27, `MessagesResendFixture.swift`) | refuse the run when the self chat has unread existing messages, is not a self chat, or the baseline cannot be digested | `selftest_begin` |
| `MessagesContentBaseline.restoreAndVerify` 43 | whether the self chat is back to its baseline (digest comparison) | `selftest_restore` |
| `MessagesCleanup.run` (261 lines: `allowsLateEcho`, `canDeleteRetracted`, `canDeleteReaction`, `removedOwnedIDs`, owned-id and baseline filters, `MessagesMediaCleanupProof`) | which rows the cleanup may delete, in what order, and whether it passed | `selftest_cleanup` |
| `MessagesAccessibility.target` 90, `validate` 212, `validateConversation` 228, `performReadState` 242, `edit` 285, `MessagesUIActions.readStateAction`, `MessagesUISession.canReplaceDraft` | UI element selection and the check that the visible conversation or message is the intended one (`target_verified`) | `ui_prepare`, `ui_validate`, `ui_perform`, `ui_activate_validate` |
| `MessagesProbes.discoverSelfHandles` 8 and `MessagesCommands.approvedHandles` 101 | the set of own handles (Contacts "me" card plus chat.db account columns) and of approved test handles | `own_handles`, `approved_handles` |
| `MessagesPins.requestedIndex` | pin index validation | UI primitives |

## 3. Primitive-only (stays in Swift)

`MessagesNativeSession` (private IMCore calls), `MessagesNavigation`, `MessagesAX`, `MessagesUISession` input and
fencing, `MessagesAppleScript.send`, the SQLite readers behind `rows_after`, `target`, `conversation`, `ledger`,
`foreign_activity`, `stage`, and the capture side (`Messages.swift`, `MessagesReader.swift`, `MessagesWatcher.swift`,
media): the capture and media files are owned by S-MSG-MEDIA and not touched here.

## 4. Removed in the accompanying draft PR

`AppleSession/MessagesSelfTest.swift` (207 lines): the step list, the argument shapes, the failed-kind proof
invalidation, the cleanup-state computation and the report. Its callers moved to `RustSelfTest`. It is the only
mechanical removal today; every row of section 1 waits for the Helper default and `ipc/1` mode.
