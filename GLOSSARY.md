# Apple Bridge

Apple Bridge makes the Apple data on one Mac available on any computer as a live, durable, read/write local copy with one REST API and one MCP server.

## Places and processes

**Gateway**:
The `bridge` program running on the Mac in gateway role; the only sequencer of bridge history and the only command authority.
_Avoid_: hub, server, master, cloud

**Replica**:
The `bridge` program on any machine holding a durable local copy of an authorized scope and serving reads locally.
_Avoid_: client, mirror, cache, slave

**Helper**:
The Swift process on the Mac that performs all Apple reads and effects.
_Avoid_: agent, daemon, adapter

**Source**:
One Apple data domain reachable through the Mac: Messages, Calendar, Reminders, Contacts, Notes or Usage.
_Avoid_: backend, provider, connector, plugin

## Data

**Bridge record**:
The bridge's identified representation of one observed Apple object, with a stable bridge ID and a version.
_Avoid_: row, native object, document, entity

**Collection**:
A named kind of bridge record, such as `reminders.reminder`.
_Avoid_: table, type

**Alias**:
A native identifier, scoped to its source generation and container, that maps to a bridge ID.
_Avoid_: external id, foreign key

**Observation**:
Evidence of a source's state, including established absence.
_Avoid_: event, notification, effect

**Capture**:
A batch of observations from one source committed atomically with its checkpoint.
_Avoid_: sync, import

**Source generation**:
One consistent native identity space of a source; a new generation resets capture identity but not bridge history.
_Avoid_: epoch

**Source checkpoint**:
Capture progress within one source generation.
_Avoid_: replication cursor, bookmark

**Complete scan**:
A capture of a declared scope whose completion evidence justifies publishing absence.
_Avoid_: full sync, refresh

**Window eviction**:
Removal of a record because it left the calendar window, which is not deletion.
_Avoid_: deletion, expiry

**Journal**:
The gateway's ordered, durable history of bridge-record changes.
_Avoid_: change log, event store, queue

**Epoch**:
One continuous journal history; a restore or other discontinuity starts a new epoch.
_Avoid_: generation

**Subscription revision**:
The version of a replica's authorized scope; a scope change starts a new revision.

**Replication cursor**:
A position `(epoch, subscription revision, sequence)` within one gateway history and scope.
_Avoid_: timestamp, offset, row id

**Snapshot**:
A consistent set of visible bridge records frozen at one journal sequence.
_Avoid_: dump, export

**Blob**:
Immutable content-addressed bytes referenced by bridge records, such as an attachment.
_Avoid_: file, attachment (when meaning the bytes)

**Record grant**:
Proof from an authorized read that a principal may open the blobs a record references.

**Reference set**:
The blobs protected from collection by records, snapshots and commands.

## Commands

**Command**:
A request for an Apple effect, identified by a caller-chosen ID, distinct from its observation.
_Avoid_: write, mutation, job

**Admission era**:
The period in which command IDs may start execution; a restore starts a new era and older IDs become lookup-only.

**Ticket**:
A single-use, expiring, session-fenced authorization for the Helper to dispatch one prepared command.

**Evidence**:
Facts recorded before and after dispatch that establish what a command did.
_Avoid_: proof, receipt

**Disposition**:
What the evidence establishes about a command: rejected before dispatch, failed with no effect, locally observed, delivered, delivery failed, indeterminate or expired.
_Avoid_: result, success flag

**Locally observed**:
The effect is visible in the Mac's own Apple state.
_Avoid_: delivered, sent successfully

**Delivered**:
Apple reports that the recipient's device received a message.

**Indeterminate**:
Whether an effect happened cannot yet be established; never retried automatically.
_Avoid_: failed, unknown-and-safe-to-retry

**Capability**:
One operation that has been proven to work on this OS build, route and object, and is therefore advertised.
_Avoid_: feature flag, support

## Quality of the copy

**Coverage**:
The records, fields, history, attachments and operations a source represents, including known gaps.
_Avoid_: completeness, synced flag

**Freshness**:
How recently a source was observed and its changes propagated.
_Avoid_: health, connectivity

## Access

**Principal**:
An authenticated caller: a paired replica or a scoped API token.
_Avoid_: user, client

**Grant**:
One permission such as `reminders:write`.

**Scope**:
The set of grants a principal holds.
_Avoid_: role, permission set

**Visibility**:
The single policy deciding which records, changes, blobs and commands a principal may see.

**Pairing**:
Establishing a replica's identity with the gateway using a one-time code.
_Avoid_: login, registration

**Delegated identity**:
A replica's signed statement of which local principal and grants a forwarded command acts for.

## Verification

**Truth**:
Apple's own state read by the harness independently of the bridge.
_Avoid_: ground truth source, expected data

**Level**:
Where a scenario runs: sim, real, replica or agent.

**Scenario**:
One end-to-end behaviour, described from outside the product and checked against Truth.
_Avoid_: test case, spec

**Feature**:
A user-visible capability tracked in the feature graph with its requirement, modules, scenarios and status.
