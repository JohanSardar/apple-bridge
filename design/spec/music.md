# Music

`music.search_catalog` uses the transient query contract with `music:read`.
Arguments: term (nonblank, at most 512 UTF-8 bytes), entity (`song`, `album`,
`musicArtist`; default song), limit (integer 1–100; default 25).
Returns `items` with string `catalog_id`, name, artist and kind.
IDs belong to the iTunes catalog; they are not Music library persistent IDs.
No token is needed. Only https://itunes.apple.com/search is contacted;
redirects are rejected, the response is bounded to 1 MiB and 20 seconds.
No artwork, previews or returned URLs are fetched. Results are not captured.

Catalog playlists are unavailable in the public API. Music automation control
and library capture require Automation: dev.applebridge.helper → com.apple.Music.
No test should request this grant. Control qualification changes Recently Played;
when authorized it must mute and restore volume and prior player state.

MUS-IDLE whole-Helper qualification depends on T53.

## Player controls

`music.now_playing` accepts an empty object and returns track, artist, album,
position, state and volume. It requires Music Automation.
`music.play`, `pause`, `next`, `previous` accept empty arguments; `set_volume`
accepts integer volume 0–100 and `play_item` accepts a captured library-item bridge ID as `id`. Catalog IDs are not accepted as library-item bridge IDs.
Every AppleScript invocation checks Automation with requesting consent disabled.
Commands compare player state after dispatch; inconclusive readback remains
indeterminate. No Music test requests consent.

## Library metadata

`music.library_item` captures name, artist, album and favorite.
`music.playlist` captures name, item references and count.
No audio or artwork blobs are captured. The native scan stops if the library
exceeds 500 tracks, 50 user playlists or 1000 playlist memberships; an incomplete
scan never publishes a complete checkpoint. Unchanged scans do not emit updates.
The same shape declarations feed the existing doctor shape check.

`music.favorite` takes `id` and boolean `favorite`; `music.add_to_playlist` takes
`id` and `playlist_id`. These are captured library-item and playlist bridge IDs. Native persistent IDs remain behind IPC.
Both commands preflight a bounded library scan and publish metadata readback
after mutation. Real control and tagged-playlist qualification remain limited
until the Helper has Music Automation; the real scenarios do not request consent.
