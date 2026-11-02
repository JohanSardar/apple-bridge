# Sending Messages with an owner token

A local bearer token created with `bridge token create` and `messages:write`
can send ordinary untagged text to the owner's self-chat. The gateway assigns
`owner_self` origin only to its own non-remote token principals for
`messages.send`. Caller-supplied origins are ignored.

The Helper verifies every recipient against discovered own handles before
Prepare and rechecks them before Dispatch. Test contacts and cached route
proofs alone do not establish ownership. Empty or mixed recipient sets fail.

Other token routes, remote tokens, lab callers and agents retain their existing
`[ab-test <id>]` tag and route gates. Agents retain levels and owner approval;
interactive device sessions retain their existing behavior. Capability proof,
expiry, dispatch fencing and conservative correlation still apply.
