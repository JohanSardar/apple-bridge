# Host native Rust readers in ab-session with a staticlib fallback

Status: Accepted subject to spike S2.

The signed Swift Helper spawns a network-free ab-session with posix_spawn and an inherited socketpair. Pre-build the same crates as a static library exposing bridge_native_main(fd) -> i32. Both use native/1; reject callback ABIs.

S2 must test the real LaunchServices and SMAppService chains, same-identity re-signing, protected files, Documents and shortcuts, with an independent launchd negative control. Never infer TCC attribution from a Terminal/ssh success. A failed child gate selects the staticlib target without changing protocol. Keep Swift identity and permission grants intact; no permission prompting in automated qualification.
