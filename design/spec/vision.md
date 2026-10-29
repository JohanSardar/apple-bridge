# Vision queries

`vision.ocr` accepts exactly one of `data_base64` or an absolute local `file` path with `allow_file: true` and optional `languages` in query args. It returns `text`, `language` (BCP-47 or `und`) and `lines` containing text, confidence and normalized Vision boxes with a bottom-left origin. Vision performs accurate recognition with automatic language detection. Encoded images are limited to 8 MiB and decoded geometry to 40 million pixels.

`pdf.text` accepts exactly one of `data_base64` or an absolute local `file` path with `allow_file: true` and returns one-based `pages` and `page_count`. `pdf.search` additionally accepts nonempty `text` and returns case-insensitive matches with one-based pages, UTF-16 offsets and lengths. Locked documents reject. Documents are limited to 8 MiB, 1,000 pages, 200,000 extracted UTF-8 bytes and 10,000 matches.

`vision.ocr`, `pdf.text` and `pdf.search` also accept `blob: {query_id, digest}` as the sole document input. The gateway resolves only an unexpired query blob lease owned by the caller with its original read grant still held, bounds the bytes to 8 MiB, and forwards inline bytes to the Helper. A digest alone grants no access. Blob inputs never expose a gateway file path.

These are transient queries. They declare no replicated collections, retain no baseline, schedule no idle polling and require no TCC grants. Gateway authorization uses separate `vision:read` and `pdf:read` scopes; corresponding write scopes are reserved. Evidence records assertions only. Synthetic fixture generation is reproducible with `xcrun swift lab/fixtures/vision/generate.swift "$PWD/lab/fixtures/vision"`.

The simulator recognizes only the generated fixtures and returns unavailable for other valid inputs. It never performs OCR. Native recognition uses Vision, NaturalLanguage and PDFKit. No third-party implementation was copied.

Work remaining: `pdf.annotate`, doctor readiness and the idle scenario. T35 and T36 are not implemented by this slice.
