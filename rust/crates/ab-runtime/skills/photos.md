# Apple Bridge: photos

Photos is read live from the Mac's library. It has no text search and is not part of `apple_search` or `bridge.list_records`.

## Finding photos

- `photos.search` with filters: `start`, `end`, `type`, `album_id`, `favorite`, `near` and `radius_m`. Follow `next_cursor` by passing it as `cursor` with the same filters, even when a page is empty.
- Every asset from `photos.search` and `photos_find` has `place`, such as "Mykonos, Greece" or "Chicago, Illinois, United States", when the photo has a location.
- Where were photos taken: `photos_find` with `places: true` (or `photos.list_places`) and `start`/`end` lists each place with its city, US state, country, dates, photo count and one `asset_id`, plus `countries` and `us_states` rollups. For a photo from every place, show each place's `asset_id` with `photos_show`. If `complete` is false, Apple's place lookup paused; call again a minute later for the rest.
- `photos.list_albums` lists albums; `photos.get` shows one asset's details and resources.
- Use `photos_find` to see matches as individual, full-frame photos. Each result's `image` points to its attached image, counted from one. Use `photos.preview_grid` with selected IDs only when you need to scan many photos yourself.

## Showing and sending photos

- Show the person the individual images from `photos_find` as they are. Keep any numbered grid out of the reply.
- For full resolution, pass the `asset_id` to `photos_show`. It returns the whole photo as Photos shows it.
- To send a photo in Messages, pass its `asset_id` in `photos` to `messages_send`; the photo file is attached as Photos shows it, as if dragged in from Photos.

## Downloading originals

`photos.download` with `asset_id` starts a transfer; call it again with `transfer_id`, `offset` and `length` to read chunks of up to 512 KiB until ready, check the SHA-256, then always call `photos.release_download`. Pick a resource from the `resources` that `photos.get` lists.

## Organizing

`photos.favorite`, `photos.create_album`, `photos.rename_album`, `photos.add_to_album`, `photos.remove_from_album` and `photos.delete_album` run through `apple_change`. Pass a stable `request_id`; repeating it never acts twice. If the result is awaiting_approval, explain the change to the owner; after they approve it, repeat the identical call once. Asset deletion is not available.
