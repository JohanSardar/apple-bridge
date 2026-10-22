# Messages send variants

`messages.send` accepts one optional `variant`. It requires an existing `chat_id`; variants never fall back to ordinary text or attachment delivery. Agent send policy excludes variants. The variant text limit is 512 UTF-16 units and formatting is bounded to 12 menu actions.

| kind | fields | capability |
| --- | --- | --- |
| schedule | scheduled_for, RFC 3339 future time within 14 days | messages.send_later |
| effect | effect, canonical bubble or screen effect | messages.send_effect |
| formatting | formatting, UTF-16 ranges with styles or text effects | messages.send_formatted |
| mention | handle_id, utf16_location, utf16_length | messages.send_mention |
| audio | exactly one file and no text | messages.send_audio |

These are capability keys for nested send variants, not separate send operations. `messages.keep_audio` is a separate version-checked message operation. Invalid or unproven requests reject before dispatch, including in SimApple. A plain-send proof never proves any variant.

The signed Helper's `selftest messages-variants --report <private-path>` selects only an existing current-build proven chat whose sole participant is an own handle. It uses unique `[ab-test …]` content, the dispatch guard, actual capture readback and exact-tag cleanup. The private proof ledger is valid only for its exact OS build, a successful exact-tag cleanup pass, tested route and tested effect or formatting parameters. An indeterminate attempt retains cleanup reconciliation. No consent dialog is automated or microphone permission requested.

Only capture-verified effect and formatting routes can currently become proven. Scheduling stops when native date semantics are unverified. Mentions stop when suggestion identity cannot be bound. Audio stops without existing microphone consent or a verified native audio-message flag. Keep stops without an owned expiring-audio fixture and kept-state readback. Those controls remain hidden; accepting their argument shape does not advertise native support.

The simulator and browser scenarios verify fail-closed coverage and absence of unproven composer controls. They do not certify native capabilities. Per-build self-test results remain private and supply the exact limited reason in source coverage.

## Current-build result

On macOS 26.7.1 (25G241), the signed own-handle batch left all six controls hidden:

| Capability | Limited reason |
| --- | --- |
| Send Later | messageValueRejected: native composer value could not be established |
| Send with effect | targetUnavailable: exact native message target could not be validated |
| Formatting | targetUnavailable: exact native message target could not be validated |
| Mention | targetUnavailable: exact native message target could not be validated |
| Audio message | audioConsentPending: existing microphone authorization was absent; no prompt requested |
| Keep | keptReadbackUnavailable: no verified kept-state readback or owned expiring-audio fixture |

Exact-tag cleanup found zero active remainder. Conservative reconciliation remains registered for UI failures; no further real mutations are queued. There are no proven composer extras to expose on this build. Simulator and browser coverage cannot promote these results.
