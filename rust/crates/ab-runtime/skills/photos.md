# Apple Bridge: photos

Photos is read live from the Mac's library. It has no text search and is not part of `apple_search` or `records.query`.

## Finding photos

- `photos.search` with filters: `start`, `end`, `type`, `album_id`, `favorite`, `near` and `radius_m`. Follow `next_cursor` by passing it as `cursor` with the same filters, even when a page is empty.
- `photos.albums` lists albums; `photos.get` shows one asset's details and resources.
- To find a photo by what it shows, look at it: `photos_find` (or `photos.contact_sheet` with about 20 IDs) returns a numbered grid of small thumbnails; read the numbers against `cells`. Images come back as image content. Each image costs time and tokens and its pixels go to your provider, so look only at what the task needs.

## Showing and sending photos

- The numbered grid and `photos.preview` are your own low-resolution index. Never show them to the person or describe their blur, crop or numbers as the photo.
- To show a photo, call `photos_show` with its `asset_id`: it returns the photo itself, full frame and full resolution as Photos shows it, with nothing drawn on it. Show that image as it is.
- `photos_show` with `save: true` also saves the original file on the Mac and returns `file`, its path, for desktop agents to open or attach.
- To send a photo in Messages, pass its `asset_id` in `photos` to `messages_send`; the original file is attached, as if dragged in from Photos.

## Downloading originals

`photos.fetch` with `asset_id` starts a transfer; call it again with `transfer_id`, `offset` and `length` to read chunks of up to 512 KiB until ready, check the SHA-256, then always call `photos.release`. Pick a resource from the `resources` that `photos.get` lists.

## Organizing

`photos.favorite`, `photos.album_create`, `photos.album_rename`, `photos.album_add`, `photos.album_remove` and `photos.album_delete` run through `apple_change`. Pass a stable `request_id`; repeating it never acts twice. If the result is awaiting_approval, explain the change to the owner; after they approve it, repeat the identical call once. Asset deletion is not available.
