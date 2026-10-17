# One authority, equal replicas

Apple stays authoritative for its data; the Mac gateway is the only sequencer of bridge history and the only command authority, and every other machine is an equal replica of the same binary. We rejected CRDT multi-writer merging and a direct-iCloud connector for v1 because Apple side effects cannot be merged safely and only the Mac sees all accounts, including local ones.
