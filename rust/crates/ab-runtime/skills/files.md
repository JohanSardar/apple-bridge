# Apple Bridge: files and documents

## Files on the Mac

- `files.search` finds files by name or content in the folders the owner shared; `files.read` reads a text file.
- `pdf.text` and `pdf.search` read PDFs; `vision.ocr` reads text in images; `audio.transcribe` transcribes audio.

## Bytes through MCP

Results that refer to a file carry a digest with a query_id or record_id. Fetch the bytes with `apple_read`, operation blobs.read, passing `digest` and the ID. Each call returns up to 512 KiB as base64; repeat with a larger `offset` until `done` is true.

To upload, use `apple_change` with operation blobs.stage and `data_base64`. It returns an `upload_id` to attach to `messages.send` as `files[].upload_id`.
