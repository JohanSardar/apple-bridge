# Frozen media goldens

The videos, still and audio here are the canonical SimApple media. They are checked in as bytes and guarded by `SHA256SUMS`; the `frozen_media_assets_match_their_recorded_digests` test fails if any file changes or a new file is unlisted.

They were first produced on macOS with AVFoundation from the inputs in `inputs/` (`still.png` is the 360x360 demo frame, `silence.m4a` is `audio.m4a` plus an 8-byte `free` box). Regenerating with `swift rust/lab/lab-simapple/generate_media.swift` needs macOS and does not reproduce these bytes: on macOS 26.5.2 the landscape clip comes out 3009 bytes instead of 3059, the live clip 3341 instead of 3343 and the still 26547 instead of 22821, and the portrait and panorama clips keep their size but differ in content. Do not regenerate unless the fixtures that embed these digests are updated with them.

`video-default.mp4` (the 16x16 fallback) has no generator in the tree; it is only a golden.
