# Third-party notices

## ThumbHash

`native/Sources/MediaDerivatives/ThumbHash.swift` and `rust/lab/lab-simapple/src/media.rs` port the encoder from
[Evan Wallace's ThumbHash reference](https://github.com/evanw/thumbhash/blob/main/js/thumbhash.js).
The codec fixtures compare against that reference, including transparent pixels.

MIT License

Copyright (c) 2023 Evan Wallace

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

## Synthetic simulation clips

`rust/lab/lab-simapple/src/media.rs` contains a one-second H.264 blue frame and a
one-second AAC silence fixture, authored with Apple's system encoders by
`MediaChecks`. The Rust lab uses only its own code to produce synthetic source
containers, image variants, placeholders and the canonical video/audio variants.
Sim video/audio use these fixed codec fixtures rather than transcoding inputs.

## iMCP

[iMCP](https://github.com/loopwork/iMCP) at commit `1f3798de359cc9e5a83b551c4390b017fb33979d` is the reference for the source ports tracked in [the port map](research/imcp-port-map.md). Research references alone do not mean a package is bundled. The existing copied ThumbHash encoder is attributed above; iMCP dependencies remain subject to their own notices when copied.

The Calendar/Reminders sim date fixtures in the Rust lab adapters under `rust/lab/lab-adapters/src` adapt the date-only, local-time and zoned-time cases from iMCP’s `App/Extensions/Foundation+Extensions.swift` and Calendar/Reminders services at the pinned commit above. DST and month-end expectations are Bridge regression fixtures.

The voicemail archive shape and UUID-addressed asset layout in `rust/crates/ab-native/src/telephony.rs` follow [`App/Services/VoicemailStore.swift`](https://github.com/mattt/iMCP/blob/c30e84cef24e2554ba5e128b1b8280e09586d5b4/App/Services/VoicemailStore.swift) at commit `c30e84cef24e2554ba5e128b1b8280e09586d5b4`, under the same MIT license below. Rust reuses Bridge's bounded plist resolver rather than instantiating archived native objects.

MIT License

Copyright 2025 Mattt (https://mat.tt)

Permission is hereby granted, free of charge, to any person obtaining a
copy of this software and associated documentation files (the "Software"),
to deal in the Software without restriction, including without limitation
the rights to use, copy, modify, merge, publish, distribute, sublicense,
and/or sell copies of the Software, and to permit persons to whom the
Software is furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in
all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS
OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
DEALINGS IN THE SOFTWARE.

## smux

The Rust relay stream multiplexer (`rust/crates/ab-remote/src/smux.rs`) implements the smux v2 wire format defined by `github.com/xtaci/smux` v1.5.57, the original Go compatibility reference, under the MIT license. Copyright (c) 2016 xtaci. Source and license: https://github.com/xtaci/smux/blob/v1.5.57/LICENSE.

MIT License

Copyright (c) 2016-2017 xtaci

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

## QR rendering

The pairing QR code is rendered with the `qrcode` crate (MIT OR Apache-2.0); `cargo deny check` enforces the dependency license policy for every Rust dependency.
