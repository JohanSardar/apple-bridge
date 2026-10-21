# Third-party notices

## ThumbHash

`native/Sources/MediaDerivatives/ThumbHash.swift` and `lab/simapple/media.go` port the encoder from
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

`lab/simapple/media_clips.go` contains a one-second H.264 blue frame and a
one-second AAC silence fixture, authored with Apple's system encoders by
`MediaChecks`. Go uses only its standard library to produce synthetic source
containers, image variants, placeholders and the canonical video/audio variants.
Sim video/audio use these fixed codec fixtures rather than transcoding inputs.
