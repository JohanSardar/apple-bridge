# Typed duplex Apple seam with SimApplePeer as second adapter

The Helper and the gateway talk over an authenticated Unix-socket conversation (Open/Ready/Capture/Committed/Prepare/Dispatch/Outcome/Resolve/BlobRead) in which Swift owns capture choreography and Go learns only durable facts and evidence. A fine-grained resource RPC was rejected because every Apple quirk would leak into Go. The same protocol is spoken by SimApplePeer, which makes sim-level E2E run the real gateway, storage, network and replicas.
