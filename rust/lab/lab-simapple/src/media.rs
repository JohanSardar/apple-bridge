#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::cast_lossless,
    clippy::many_single_char_names,
    clippy::similar_names,
    clippy::manual_midpoint
)]
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub(super) struct RgbaImage {
    width: u32,
    height: u32,
    data: Vec<u8>,
}

impl RgbaImage {
    fn from_fn(width: u32, height: u32, mut f: impl FnMut(u32, u32) -> [u8; 4]) -> Self {
        let mut data = Vec::with_capacity(width as usize * height as usize * 4);
        for y in 0..height {
            for x in 0..width {
                data.extend_from_slice(&f(x, y));
            }
        }
        Self {
            width,
            height,
            data,
        }
    }
    fn from_pixels(width: u32, height: u32, data: Vec<u8>) -> Option<Self> {
        (data.len() == width as usize * height as usize * 4 && width > 0 && height > 0).then_some(
            Self {
                width,
                height,
                data,
            },
        )
    }
    fn width(&self) -> u32 {
        self.width
    }
    fn height(&self) -> u32 {
        self.height
    }
    fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }
    fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let at = (y as usize * self.width as usize + x as usize) * 4;
        [
            self.data[at],
            self.data[at + 1],
            self.data[at + 2],
            self.data[at + 3],
        ]
    }
    fn pixels(&self) -> impl Iterator<Item = &[u8; 4]> {
        self.data.as_chunks::<4>().0.iter()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Format {
    Png,
    Jpeg,
    Gif,
}

fn expand(buffer: &[u8], space: zune_core::colorspace::ColorSpace) -> Option<Vec<u8>> {
    use zune_core::colorspace::ColorSpace;
    match space {
        ColorSpace::RGBA => Some(buffer.to_vec()),
        ColorSpace::RGB => Some(
            buffer
                .as_chunks::<3>()
                .0
                .iter()
                .flat_map(|p| [p[0], p[1], p[2], 255])
                .collect(),
        ),
        ColorSpace::LumaA => Some(
            buffer
                .as_chunks::<2>()
                .0
                .iter()
                .flat_map(|p| [p[0], p[0], p[0], p[1]])
                .collect(),
        ),
        ColorSpace::Luma => Some(buffer.iter().flat_map(|v| [*v, *v, *v, 255]).collect()),
        _ => None,
    }
}

fn decode_image(data: &[u8]) -> Option<(Format, RgbaImage)> {
    use zune_core::{
        bytestream::ZCursor, colorspace::ColorSpace, options::DecoderOptions,
        result::DecodingResult,
    };
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        let options = DecoderOptions::default().png_set_strip_to_8bit(true);
        let mut decoder = zune_png::PngDecoder::new_with_options(ZCursor::new(data), options);
        let DecodingResult::U8(buffer) = decoder.decode().ok()? else {
            return None;
        };
        let (width, height) = decoder.dimensions()?;
        let pixels = expand(&buffer, decoder.colorspace()?)?;
        return Some((
            Format::Png,
            RgbaImage::from_pixels(
                u32::try_from(width).ok()?,
                u32::try_from(height).ok()?,
                pixels,
            )?,
        ));
    }
    if data.starts_with(&[0xff, 0xd8, 0xff]) {
        let options = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGB);
        let mut decoder = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(data), options);
        let buffer = decoder.decode().ok()?;
        let info = decoder.info()?;
        let pixels = expand(&buffer, ColorSpace::RGB)?;
        return Some((
            Format::Jpeg,
            RgbaImage::from_pixels(u32::from(info.width), u32::from(info.height), pixels)?,
        ));
    }
    if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        let mut options = gif::DecodeOptions::new();
        options.set_color_output(gif::ColorOutput::RGBA);
        let mut decoder = options.read_info(std::io::Cursor::new(data)).ok()?;
        let frame = decoder.read_next_frame().ok()??;
        return Some((
            Format::Gif,
            RgbaImage::from_pixels(
                u32::from(frame.width),
                u32::from(frame.height),
                frame.buffer.to_vec(),
            )?,
        ));
    }
    None
}

fn encode_png(image: &RgbaImage) -> Vec<u8> {
    fn chunk(out: &mut Vec<u8>, kind: [u8; 4], body: &[u8]) {
        out.extend_from_slice(&(body.len() as u32).to_be_bytes());
        let mut checked = kind.to_vec();
        checked.extend_from_slice(body);
        out.extend_from_slice(&checked);
        out.extend_from_slice(&crc32fast::hash(&checked).to_be_bytes());
    }
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut header = Vec::new();
    header.extend_from_slice(&image.width.to_be_bytes());
    header.extend_from_slice(&image.height.to_be_bytes());
    header.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut out, *b"IHDR", &header);
    let stride = image.width as usize * 4;
    let mut raw = Vec::with_capacity((stride + 1) * image.height as usize);
    for row in image.data.chunks_exact(stride) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    chunk(
        &mut out,
        *b"IDAT",
        &miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6),
    );
    chunk(&mut out, *b"IEND", &[]);
    out
}

const VIDEO_DEFAULT: &[u8] = include_bytes!("../assets/video-default.mp4");
const VIDEO_LANDSCAPE: &[u8] = include_bytes!("../assets/video-landscape.mp4");
const VIDEO_PORTRAIT: &[u8] = include_bytes!("../assets/video-portrait.mp4");
const VIDEO_PANORAMA: &[u8] = include_bytes!("../assets/video-panorama.mp4");
const VIDEO_LIVE: &[u8] = include_bytes!("../assets/video-live.mp4");
const LIVE_STILL: &[u8] = include_bytes!("../assets/live-still.png");
const AUDIO: &[u8] = include_bytes!("../assets/audio.m4a");
const FREE_BOX: [u8; 8] = [0, 0, 0, 8, b'f', b'r', b'e', b'e'];

#[derive(Debug)]
pub(super) struct Derived {
    pub derivatives: Value,
    pub media: Value,
    pub blobs: BTreeMap<String, Vec<u8>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Geometry {
    pub width: i64,
    pub height: i64,
    pub duration_ms: i64,
}

pub(super) fn fixture(kind: &str) -> Option<Vec<u8>> {
    let raw = match kind {
        "video" => VIDEO_LANDSCAPE,
        "video-portrait" => VIDEO_PORTRAIT,
        "video-panorama" => VIDEO_PANORAMA,
        "video-live" => VIDEO_LIVE,
        "live-still" => return Some(LIVE_STILL.to_vec()),
        "audio" => AUDIO,
        _ => return None,
    };
    let mut data = raw.to_vec();
    data.extend_from_slice(&FREE_BOX);
    Some(data)
}

pub(super) fn digest(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

fn extension(name: &str) -> String {
    name.rsplit_once('.')
        .map(|(_, e)| format!(".{}", e.to_lowercase()))
        .unwrap_or_default()
}

fn mp4_sniff(data: &[u8]) -> bool {
    if data.len() < 12 || &data[4..8] != b"ftyp" {
        return false;
    }
    let size = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
    if data.len() < size || !size.is_multiple_of(4) {
        return false;
    }
    (8..size)
        .step_by(4)
        .filter(|start| *start != 12)
        .any(|start| &data[start..start + 3] == b"mp4")
}

pub(super) fn mime(name: &str, data: &[u8]) -> &'static str {
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        return "image/png";
    }
    if data.starts_with(&[0xff, 0xd8, 0xff]) {
        return "image/jpeg";
    }
    if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        return "image/gif";
    }
    if mp4_sniff(data) {
        return "video/mp4";
    }
    match extension(name).as_str() {
        ".caf" | ".m4a" | ".mp3" | ".wav" => "audio/mp4",
        ".mov" | ".mp4" => "video/mp4",
        _ => "application/octet-stream",
    }
}

fn resize_to(input: &RgbaImage, width: u32, height: u32) -> RgbaImage {
    let (iw, ih) = input.dimensions();
    RgbaImage::from_fn(width, height, |x, y| {
        input.pixel(
            (u64::from(x) * u64::from(iw) / u64::from(width)) as u32,
            (u64::from(y) * u64::from(ih) / u64::from(height)) as u32,
        )
    })
}

pub(super) fn resize(input: &RgbaImage, maximum: u32) -> RgbaImage {
    let (w, h) = input.dimensions();
    let scale = f64::min(1.0, f64::from(maximum) / f64::from(w.max(h)));
    resize_to(
        input,
        ((f64::from(w) * scale) as u32).max(1),
        ((f64::from(h) * scale) as u32).max(1),
    )
}

pub(super) fn poster_dimensions(width: i64, height: i64) -> (i64, i64) {
    if width <= 0 || height <= 0 {
        return (0, 0);
    }
    let ratio = width.max(height) as f64 / width.min(height) as f64;
    let short = ((1_500_000.0 / ratio).sqrt().floor() as i64).clamp(1, 360);
    let long = ((short as f64 * ratio).min(1_500_000.0).floor() as i64).max(1);
    if width >= height {
        (long, short)
    } else {
        (short, long)
    }
}

fn poster_resize(input: &RgbaImage) -> RgbaImage {
    let (w, h) = poster_dimensions(i64::from(input.width()), i64::from(input.height()));
    resize_to(input, w as u32, h as u32)
}

fn gcd(mut a: i64, mut b: i64) -> i64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

fn poster_at(w: i64, h: i64, source: bool) -> RgbaImage {
    let side = w.min(h) * 4 / 5;
    RgbaImage::from_fn(w as u32, h as u32, |x, y| {
        let (x, y) = (i64::from(x), i64::from(y));
        if source {
            if x >= (w - side) / 2
                && x < (w + side) / 2
                && y >= (h - side) / 2
                && y < (h + side) / 2
            {
                [255, 0, 0, 255]
            } else {
                [0, 76, 200, 255]
            }
        } else {
            [40, 120, 200, 255]
        }
    })
}

fn legacy_poster(width: i64, height: i64, source: bool) -> RgbaImage {
    let divisor = gcd(width, height).max(1);
    let (mut w, mut h) = (width / divisor, height / divisor);
    let factor = divisor.min(320 / w.max(h));
    if factor > 0 {
        w *= factor;
        h *= factor;
    } else {
        let long = width.max(height);
        w = (width * 320 / long).max(1);
        h = (height * 320 / long).max(1);
    }
    poster_at(w, h, source)
}

fn opaque(image: &RgbaImage) -> bool {
    image.pixels().all(|p| p[3] == 255)
}

fn encode(image: &RgbaImage, quality: u8) -> Option<(Vec<u8>, &'static str)> {
    if opaque(image) {
        let rgb: Vec<u8> = image.pixels().flat_map(|p| [p[0], p[1], p[2]]).collect();
        let mut buffer = Vec::new();
        jpeg_encoder::Encoder::new(&mut buffer, quality)
            .encode(
                &rgb,
                u16::try_from(image.width).ok()?,
                u16::try_from(image.height).ok()?,
                jpeg_encoder::ColorType::Rgb,
            )
            .ok()?;
        Some((buffer, "image/jpeg"))
    } else {
        Some((encode_png(image), "image/png"))
    }
}

pub(super) fn thumbhash(input: &RgbaImage) -> String {
    let (w, h) = (input.width() as usize, input.height() as usize);
    let count = w * h;
    let mut average = [0f64; 4];
    for p in input.pixels() {
        let a = f64::from(p[3]) / 255.0;
        for (value, channel) in average.iter_mut().zip(p.iter()).take(3) {
            *value += a / 255.0 * f64::from(*channel);
        }
        average[3] += a;
    }
    if average[3] > 0.0 {
        let total = average[3];
        for value in &mut average[..3] {
            *value /= total;
        }
    }
    let alpha = average[3] < count as f64;
    let limit = if alpha { 5.0 } else { 7.0 };
    let longest = w.max(h) as f64;
    let lx = ((limit * w as f64 / longest).round() as usize).max(1);
    let ly = ((limit * h as f64 / longest).round() as usize).max(1);
    let mut channels = vec![vec![0f64; count]; 4];
    for (i, p) in input.pixels().enumerate() {
        let a = f64::from(p[3]) / 255.0;
        let r = average[0] * (1.0 - a) + a / 255.0 * f64::from(p[0]);
        let g = average[1] * (1.0 - a) + a / 255.0 * f64::from(p[1]);
        let b = average[2] * (1.0 - a) + a / 255.0 * f64::from(p[2]);
        channels[0][i] = (r + g + b) / 3.0;
        channels[1][i] = (r + g) / 2.0 - b;
        channels[2][i] = r - g;
        channels[3][i] = a;
    }
    let encode_channel = |values: &[f64], nx: usize, ny: usize| {
        let (mut dc, mut scale, mut ac) = (0.0, 0f64, Vec::new());
        for cy in 0..ny {
            let mut cx = 0;
            while cx * ny < nx * (ny - cy) {
                let mut value = 0.0;
                for y in 0..h {
                    let fy = (std::f64::consts::PI / h as f64 * cy as f64 * (y as f64 + 0.5)).cos();
                    for x in 0..w {
                        value += values[x + y * w]
                            * (std::f64::consts::PI / w as f64 * cx as f64 * (x as f64 + 0.5))
                                .cos()
                            * fy;
                    }
                }
                value /= count as f64;
                if cx != 0 || cy != 0 {
                    ac.push(value);
                    scale = scale.max(value.abs());
                } else {
                    dc = value;
                }
                cx += 1;
            }
        }
        if scale > 0.0 {
            for v in &mut ac {
                *v = 0.5 + 0.5 / scale * *v;
            }
        }
        (dc, scale, ac)
    };
    let l = encode_channel(&channels[0], lx.max(3), ly.max(3));
    let p = encode_channel(&channels[1], 3, 3);
    let q = encode_channel(&channels[2], 3, 3);
    let a = if alpha {
        encode_channel(&channels[3], 5, 5)
    } else {
        (0.0, 0.0, Vec::new())
    };
    let round = |v: f64| -> i64 { (v + 0.5).floor() as i64 };
    let mut h24 = round(63.0 * l.0)
        | (round(31.5 + 31.5 * p.0) << 6)
        | (round(31.5 + 31.5 * q.0) << 12)
        | (round(31.0 * l.1) << 18);
    if alpha {
        h24 |= 1 << 23;
    }
    let short = if w > h { ly } else { lx } as i64;
    let mut h16 = short | (round(63.0 * p.1) << 3) | (round(63.0 * q.1) << 9);
    if w > h {
        h16 |= 1 << 15;
    }
    let mut hash = vec![
        h24 as u8,
        (h24 >> 8) as u8,
        (h24 >> 16) as u8,
        h16 as u8,
        (h16 >> 8) as u8,
    ];
    let mut values = vec![&l, &p, &q];
    if alpha {
        hash.push((round(15.0 * a.0) | (round(15.0 * a.1) << 4)) as u8);
        values.push(&a);
    }
    let start = hash.len();
    let mut index = 0;
    for channel in values {
        for v in &channel.2 {
            let position = start + index / 2;
            if position == hash.len() {
                hash.push(0);
            }
            hash[position] |= (round(15.0 * v) << ((index & 1) * 4)) as u8;
            index += 1;
        }
    }
    super::query::base64(&hash)
}

fn boxes<'a>(mut data: &'a [u8], mut visit: impl FnMut(&[u8], &'a [u8])) -> bool {
    while !data.is_empty() {
        if data.len() < 8 {
            return false;
        }
        let (mut size, mut header) = (
            u64::from(u32::from_be_bytes([data[0], data[1], data[2], data[3]])),
            8usize,
        );
        if size == 1 {
            if data.len() < 16 {
                return false;
            }
            let mut wide = [0u8; 8];
            wide.copy_from_slice(&data[8..16]);
            size = u64::from_be_bytes(wide);
            header = 16;
        }
        if size == 0 {
            size = data.len() as u64;
        }
        if size < header as u64 || size > data.len() as u64 {
            return false;
        }
        visit(&data[4..8], &data[header..size as usize]);
        data = &data[size as usize..];
    }
    true
}

fn be32(data: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]])
}

fn be64(data: &[u8], at: usize) -> u64 {
    let mut wide = [0u8; 8];
    wide.copy_from_slice(&data[at..at + 8]);
    u64::from_be_bytes(wide)
}

fn duration(data: &[u8]) -> i64 {
    if data.len() < 20 {
        return 0;
    }
    let mut offset = 12;
    if data[0] == 1 {
        offset = 20;
        if data.len() < 32 {
            return 0;
        }
    } else if data[0] != 0 {
        return 0;
    }
    let scale = u64::from(be32(data, offset));
    let ticks = if data[0] == 1 {
        be64(data, offset + 4)
    } else {
        let t = u64::from(be32(data, offset + 4));
        if t == u64::from(u32::MAX) {
            return 0;
        }
        t
    };
    let limit = i64::MAX as u64;
    if scale == 0 || ticks == u64::MAX || ticks / scale > limit / 1000 {
        return 0;
    }
    let value = ticks / scale * 1000 + ticks % scale * 1000 / scale;
    if value > limit { 0 } else { value as i64 }
}

fn dimensions(data: &[u8]) -> (i64, i64) {
    if data.len() < 84 {
        return (0, 0);
    }
    let (matrix, end) = if data[0] == 1 {
        if data.len() < 96 {
            return (0, 0);
        }
        (52, 96)
    } else if data[0] == 0 {
        (40, 84)
    } else {
        return (0, 0);
    };
    let width = f64::from(be32(data, end - 8)) / 65536.0;
    let height = f64::from(be32(data, end - 4)) / 65536.0;
    let mut values = [0f64; 4];
    for (i, offset) in [0, 4, 12, 16].into_iter().enumerate() {
        values[i] = f64::from(be32(data, matrix + offset) as i32) / 65536.0;
    }
    let x = (values[0].abs() * width + values[2].abs() * height).round();
    let y = (values[1].abs() * width + values[3].abs() * height).round();
    if x < 1.0 || y < 1.0 || x > 1_048_576.0 || y > 1_048_576.0 {
        return (0, 0);
    }
    (x as i64, y as i64)
}

pub(super) fn source_geometry(data: &[u8]) -> Option<Geometry> {
    let mut movie: &[u8] = &[];
    if !boxes(data, |kind, body| {
        if kind == b"moov" {
            movie = body;
        }
    }) || movie.is_empty()
    {
        return None;
    }
    let mut clock: &[u8] = &[];
    let mut tracks: Vec<&[u8]> = Vec::new();
    if !boxes(movie, |kind, body| {
        if kind == b"mvhd" {
            clock = body;
        }
        if kind == b"trak" && tracks.len() < 128 {
            tracks.push(body);
        }
    }) {
        return None;
    }
    let mut selected = Geometry {
        width: 0,
        height: 0,
        duration_ms: 0,
    };
    for track in tracks {
        let (mut header, mut media): (&[u8], &[u8]) = (&[], &[]);
        if !boxes(track, |kind, body| {
            if kind == b"tkhd" {
                header = body;
            }
            if kind == b"mdia" {
                media = body;
            }
        }) {
            continue;
        }
        let (mut handler, mut timing): (&[u8], &[u8]) = (&[], &[]);
        if !boxes(media, |kind, body| {
            if kind == b"hdlr" {
                handler = body;
            }
            if kind == b"mdhd" {
                timing = body;
            }
        }) || handler.len() < 12
            || &handler[8..12] != b"vide"
        {
            continue;
        }
        let (width, height) = dimensions(header);
        let mut length = duration(clock);
        if length == 0 {
            length = duration(timing);
        }
        if width > 0
            && height > 0
            && length > 0
            && width * height > selected.width * selected.height
        {
            selected = Geometry {
                width,
                height,
                duration_ms: length,
            };
        }
    }
    (selected.width > 0).then_some(selected)
}

struct Store {
    blobs: BTreeMap<String, Vec<u8>>,
    derivatives: Map<String, Value>,
}

impl Store {
    fn save(&mut self, name: &str, mime: &str, data: Vec<u8>, size: Option<(i64, i64)>) {
        let key = digest(&data);
        let mut variant = json!({"digest":key,"state":"present","size":data.len(),"mime":mime});
        if let Some((width, height)) = size.filter(|(w, _)| *w > 0) {
            variant["width"] = json!(width);
            variant["height"] = json!(height);
        }
        self.blobs.insert(key, data);
        self.derivatives.insert(name.to_owned(), variant);
    }
    fn poster(&mut self, poster: &RgbaImage) -> Option<()> {
        let (data, mime) = encode(poster, 80)?;
        let size = (i64::from(poster.width()), i64::from(poster.height()));
        self.save("poster", mime, data, Some(size));
        Some(())
    }
}

pub(super) fn derive(name: &str, original: &[u8]) -> Derived {
    let mut store = Store {
        blobs: BTreeMap::new(),
        derivatives: Map::new(),
    };
    let mut media = Map::new();
    if let Some((format, input)) = decode_image(original) {
        media.insert("width".into(), json!(input.width()));
        media.insert("height".into(), json!(input.height()));
        media.insert("thumbhash".into(), json!(thumbhash(&resize(&input, 100))));
        for (variant, maximum, quality) in [("thumb", 400, 80), ("display", 1600, 85)] {
            if format == Format::Gif && variant == "display" {
                continue;
            }
            let output = resize(&input, maximum);
            if let Some((data, mime)) = encode(&output, quality) {
                let size = (i64::from(output.width()), i64::from(output.height()));
                store.save(variant, mime, data, Some(size));
            }
        }
    } else {
        match extension(name).as_str() {
            ".mov" | ".mp4" | ".3gp" => derive_video(original, &mut store, &mut media),
            ".caf" | ".m4a" | ".mp3" | ".amr" | ".wav" => {
                store.save("audio", "audio/mp4", AUDIO.to_vec(), None);
                media.insert("waveform".into(), json!(vec![0; 64]));
                media.insert("duration_ms".into(), json!(1000));
            }
            _ => {}
        }
    }
    Derived {
        derivatives: Value::Object(store.derivatives),
        media: Value::Object(media),
        blobs: store.blobs,
    }
}

fn derive_video(original: &[u8], store: &mut Store, media: &mut Map<String, Value>) {
    let parsed = source_geometry(original);
    let (clip, mut clip_w, mut clip_h, source, geometry) = match parsed {
        Some(g) if g.height > g.width => (VIDEO_PORTRAIT, 180, 320, "source", g),
        Some(g) => (VIDEO_LANDSCAPE, 320, 180, "source", g),
        None => (
            VIDEO_DEFAULT,
            16,
            16,
            "sim-default",
            Geometry {
                width: 16,
                height: 16,
                duration_ms: 1000,
            },
        ),
    };
    let mut clip = clip.to_vec();
    if original == fixture("video-panorama").unwrap_or_default().as_slice()
        || original == fixture("video-live").unwrap_or_default().as_slice()
    {
        clip = original.to_vec();
        clip_w = geometry.width;
        clip_h = geometry.height;
    }
    store.save("video", "video/mp4", clip, Some((clip_w, clip_h)));
    let valid = parsed.is_some();
    let mut legacy = legacy_poster(geometry.width, geometry.height, valid);
    let (w, h) = poster_dimensions(geometry.width, geometry.height);
    let mut poster = poster_at(w, h, valid);
    if original == fixture("video-live").unwrap_or_default().as_slice()
        && let Some((_, still)) = decode_image(LIVE_STILL)
    {
        legacy = resize(&still, 320);
        poster = poster_resize(&still);
    }
    store.poster(&poster);
    media.insert("width".into(), json!(geometry.width));
    media.insert("height".into(), json!(geometry.height));
    media.insert("duration_ms".into(), json!(geometry.duration_ms));
    media.insert("dims_source".into(), json!(source));
    media.insert("thumbhash".into(), json!(thumbhash(&resize(&legacy, 32))));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient_a() -> RgbaImage {
        RgbaImage::from_fn(40, 30, |x, y| {
            [
                (x * x + y * 3) as u8,
                (x * y) as u8,
                (x * 7 + y * 13) as u8,
                255,
            ]
        })
    }

    fn gradient_b() -> RgbaImage {
        RgbaImage::from_fn(24, 48, |x, y| {
            [
                (x * 10) as u8,
                (y * 5) as u8,
                (x * y) as u8,
                (100 + x * 6) as u8,
            ]
        })
    }

    #[test]
    fn thumbhash_matches_go_vectors() {
        assert_eq!(thumbhash(&gradient_a()), "nBcGHYYCQ4VThpiEhViEhmBSpvf4");
        assert_eq!(thumbhash(&gradient_b()), "XhiKExAqgIBoaHjxwA75hnB3eIiHiHc=");
    }

    #[test]
    fn video_geometry_and_thumbhash_match_go() {
        let cases = [
            (
                "video",
                (320, 180),
                "VmYCnKz4d1d4h4h6hngHh3VwWA==",
                (320, 180),
                (640, 360),
            ),
            (
                "video-portrait",
                (180, 320),
                "VmYCnCyoiHf3Z4d4eFhXh3B1CA==",
                (180, 320),
                (360, 640),
            ),
            (
                "video-panorama",
                (1920, 200),
                "l5MBIYr4Bul7hJtnB4R2QGg=",
                (1920, 200),
                (3456, 360),
            ),
            (
                "video-live",
                (360, 360),
                "7HcBDwYadHeDdodQmkiHiHd3CEMokHwI",
                (360, 360),
                (360, 360),
            ),
        ];
        for (kind, size, hash, clip, poster) in cases {
            let data = fixture(kind).unwrap_or_default();
            let geometry = source_geometry(&data);
            assert_eq!(
                geometry,
                Some(Geometry {
                    width: size.0,
                    height: size.1,
                    duration_ms: 1000
                }),
                "{kind}"
            );
            let derived = derive("x.mov", &data);
            assert_eq!(derived.media["thumbhash"], hash, "{kind}");
            assert_eq!(derived.media["dims_source"], "source", "{kind}");
            assert_eq!(derived.derivatives["video"]["width"], clip.0, "{kind}");
            assert_eq!(derived.derivatives["video"]["height"], clip.1, "{kind}");
            assert_eq!(derived.derivatives["poster"]["width"], poster.0, "{kind}");
            assert_eq!(derived.derivatives["poster"]["height"], poster.1, "{kind}");
            assert_eq!(
                derived.derivatives["poster"]["mime"], "image/jpeg",
                "{kind}"
            );
            assert_eq!(derived.blobs.len(), 2, "{kind}");
        }
    }

    #[test]
    fn unparsable_video_uses_simulator_defaults() {
        let derived = derive("x.mp4", b"not a movie");
        assert_eq!(derived.media["dims_source"], "sim-default");
        assert_eq!(derived.media["width"], 16);
        assert_eq!(derived.derivatives["video"]["width"], 16);
        assert_eq!(derived.derivatives["poster"]["width"], 360);
    }

    #[test]
    fn audio_derivation_and_mime_match_go() {
        let data = fixture("audio").unwrap_or_default();
        let derived = derive("x.caf", &data);
        assert_eq!(derived.derivatives["audio"]["mime"], "audio/mp4");
        assert_eq!(derived.media["duration_ms"], 1000);
        assert_eq!(derived.media["waveform"], json!(vec![0; 64]));
        assert_eq!(mime("a.caf", &data), "video/mp4");
        assert_eq!(mime("a.png", LIVE_STILL), "image/png");
        assert_eq!(mime("a.wav", b"RIFF"), "audio/mp4");
        assert_eq!(mime("a.bin", b"xx"), "application/octet-stream");
    }

    #[test]
    fn image_derivation_resizes_and_hashes() {
        let encoded = encode_png(&RgbaImage::from_fn(2400, 1200, |x, y| {
            [(x % 256) as u8, (y % 256) as u8, 90, 255]
        }));
        let derived = derive("a.png", &encoded);
        assert_eq!(derived.media["width"], 2400);
        assert_eq!(derived.media["height"], 1200);
        assert_eq!(derived.derivatives["thumb"]["width"], 400);
        assert_eq!(derived.derivatives["thumb"]["height"], 200);
        assert_eq!(derived.derivatives["display"]["width"], 1600);
        assert_eq!(derived.derivatives["display"]["mime"], "image/jpeg");
        let thumb = derived.derivatives["thumb"]["digest"]
            .as_str()
            .unwrap_or_default();
        assert_eq!(
            derived.blobs[thumb].len(),
            derived.derivatives["thumb"]["size"]
        );
        let still = derive("a.png", LIVE_STILL);
        assert_eq!(still.media["thumbhash"], "7HcBDwYadHeTdodQmkiHiHd3B1MogHwI");
    }

    #[test]
    fn alpha_image_keeps_png_variants() {
        let encoded = encode_png(&gradient_b());
        let derived = derive("b.png", &encoded);
        assert_eq!(derived.derivatives["thumb"]["mime"], "image/png");
        assert_eq!(
            derived.media["thumbhash"],
            "XhiKExAqgIBoaHjxwA75hnB3eIiHiHc="
        );
    }

    #[test]
    fn poster_dimensions_match_go() {
        assert_eq!(poster_dimensions(1920, 200), (3456, 360));
        assert_eq!(poster_dimensions(320, 180), (640, 360));
        assert_eq!(poster_dimensions(180, 320), (360, 640));
        assert_eq!(poster_dimensions(16, 16), (360, 360));
        assert_eq!(poster_dimensions(0, 5), (0, 0));
    }
}
