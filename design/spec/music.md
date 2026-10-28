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
