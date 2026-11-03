use crate::rng::Pcg;
use std::fmt::Write as _;

const VIDEO_LANDSCAPE: &[u8] = include_bytes!("../../lab-simapple/assets/video-landscape.mp4");
const VIDEO_PORTRAIT: &[u8] = include_bytes!("../../lab-simapple/assets/video-portrait.mp4");
const VIDEO_PANORAMA: &[u8] = include_bytes!("../../lab-simapple/assets/video-panorama.mp4");
const VIDEO_LIVE: &[u8] = include_bytes!("../../lab-simapple/assets/video-live.mp4");
const LIVE_STILL: &[u8] = include_bytes!("../../lab-simapple/assets/live-still.png");
const AUDIO: &[u8] = include_bytes!("../../lab-simapple/assets/audio.m4a");
const FREE_BOX: [u8; 8] = [0, 0, 0, 8, b'f', b'r', b'e', b'e'];

#[derive(Debug, Clone)]
pub struct Asset {
    pub name: &'static str,
    pub mime: &'static str,
    pub data: Vec<u8>,
}

#[must_use]
pub fn fixture(kind: &str) -> Vec<u8> {
    let raw = match kind {
        "video" => VIDEO_LANDSCAPE,
        "video-portrait" => VIDEO_PORTRAIT,
        "video-panorama" => VIDEO_PANORAMA,
        "video-live" => VIDEO_LIVE,
        "live-still" => return LIVE_STILL.to_vec(),
        "audio" => AUDIO,
        _ => return vec![],
    };
    let mut data = raw.to_vec();
    data.extend_from_slice(&FREE_BOX);
    data
}

fn chunk(out: &mut Vec<u8>, kind: [u8; 4], body: &[u8]) {
    out.extend_from_slice(&u32::try_from(body.len()).unwrap_or_default().to_be_bytes());
    let mut crc = crc32fast::Hasher::new();
    crc.update(&kind);
    crc.update(body);
    out.extend_from_slice(&kind);
    out.extend_from_slice(body);
    out.extend_from_slice(&crc.finalize().to_be_bytes());
}

fn encode_png(width: usize, height: usize, pixels: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity((width * 4 + 1) * height);
    for row in pixels.chunks(width * 4) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    let mut header = Vec::new();
    header.extend_from_slice(&u32::try_from(width).unwrap_or_default().to_be_bytes());
    header.extend_from_slice(&u32::try_from(height).unwrap_or_default().to_be_bytes());
    header.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut out, *b"IHDR", &header);
    chunk(
        &mut out,
        *b"IDAT",
        &miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6),
    );
    chunk(&mut out, *b"IEND", &[]);
    out
}

#[must_use]
#[allow(
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss
)]
pub fn picture(seed: u64, width: usize, height: usize, transparent: bool) -> Vec<u8> {
    let mut rng = Pcg::new(seed, seed ^ 0x85eb_ca6b);
    let base = [
        80.0 + rng.float64() * 100.0,
        90.0 + rng.float64() * 100.0,
        130.0 + rng.float64() * 100.0,
    ];
    let mut pixels = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        for x in 0..width {
            let u = x as f64 / width as f64;
            let v = y as f64 / height as f64;
            let shape = (-12.0 * ((u - 0.35) * (u - 0.35) + (v - 0.4) * (v - 0.4))).exp();
            let alpha = if transparent {
                255.0 * (((0.44 - (u - 0.5).hypot(v - 0.5)) * 30.0).clamp(0.0, 1.0))
            } else {
                255.0
            };
            pixels.push((base[0] + 50.0 * u + 35.0 * shape).min(255.0) as u8);
            pixels.push((base[1] + 35.0 * v + 45.0 * shape).min(255.0) as u8);
            pixels.push((base[2] + 25.0 * (1.0 - u)).min(255.0) as u8);
            pixels.push(alpha as u8);
        }
    }
    encode_png(width, height, &pixels)
}

fn lzw(indices: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut accumulator = 0_u32;
    let mut bits = 0_u32;
    let mut emit = |code: u32, width: u32, out: &mut Vec<u8>| {
        accumulator |= code << bits;
        bits += width;
        while bits >= 8 {
            out.push(u8::try_from(accumulator & 0xff).unwrap_or_default());
            accumulator >>= 8;
            bits -= 8;
        }
    };
    let clear = 4_u32;
    let end = 5_u32;
    let mut width = 3_u32;
    emit(clear, width, &mut out);
    let mut table: std::collections::HashMap<(u32, u8), u32> = std::collections::HashMap::new();
    let mut next = 6_u32;
    let mut current = u32::from(indices[0]);
    for &symbol in &indices[1..] {
        if let Some(&code) = table.get(&(current, symbol)) {
            current = code;
            continue;
        }
        emit(current, width, &mut out);
        table.insert((current, symbol), next);
        if next == (1 << width) && width < 12 {
            width += 1;
        }
        next += 1;
        if next >= 4096 {
            emit(clear, width, &mut out);
            table.clear();
            next = 6;
            width = 3;
        }
        current = u32::from(symbol);
    }
    emit(current, width, &mut out);
    emit(end, width, &mut out);
    if bits > 0 {
        out.push(u8::try_from(accumulator & 0xff).unwrap_or_default());
    }
    out
}

#[must_use]
pub fn animation() -> Vec<u8> {
    let mut out = b"GIF89a".to_vec();
    out.extend_from_slice(&96_u16.to_le_bytes());
    out.extend_from_slice(&96_u16.to_le_bytes());
    out.extend_from_slice(&[0x80, 0, 0, 90, 150, 210, 190, 130, 210]);
    out.extend_from_slice(&[0x21, 0xff, 0x0b]);
    out.extend_from_slice(b"NETSCAPE2.0");
    out.extend_from_slice(&[3, 1, 0, 0, 0]);
    for frame in 0..2_usize {
        out.extend_from_slice(&[0x21, 0xf9, 4, 0, 30, 0, 0, 0]);
        out.push(0x2c);
        out.extend_from_slice(&[0, 0, 0, 0]);
        out.extend_from_slice(&96_u16.to_le_bytes());
        out.extend_from_slice(&96_u16.to_le_bytes());
        out.push(0);
        out.push(2);
        let mut indices = Vec::with_capacity(96 * 96);
        for y in 0..96_usize {
            for x in 0..96_usize {
                indices.push(u8::try_from((x / 24 + y / 24 + frame) % 2).unwrap_or_default());
            }
        }
        for block in lzw(&indices).chunks(255) {
            out.push(u8::try_from(block.len()).unwrap_or_default());
            out.extend_from_slice(block);
        }
        out.push(0);
    }
    out.push(0x3b);
    out
}

#[must_use]
pub fn document() -> Vec<u8> {
    let content = "BT /F1 18 Tf 32 180 Td (Synthetic Demo Field Notes) Tj ET";
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 320 240] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>".to_owned(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
        format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        ),
    ];
    let mut buffer = String::from("%PDF-1.4\n");
    let mut offsets = vec![0_usize];
    for (index, object) in objects.iter().enumerate() {
        offsets.push(buffer.len());
        let _ = write!(buffer, "{} 0 obj\n{object}\nendobj\n", index + 1);
    }
    let start = buffer.len();
    let _ = write!(buffer, "xref\n0 {}\n0000000000 65535 f \n", offsets.len());
    for offset in &offsets[1..] {
        let _ = writeln!(buffer, "{offset:010} 00000 n ");
    }
    let _ = write!(
        buffer,
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{start}\n%%EOF\n",
        offsets.len()
    );
    buffer.into_bytes()
}

#[must_use]
pub fn assets(rng: &mut Pcg) -> Vec<Asset> {
    let mut png = |name, width, height, transparent| Asset {
        name,
        mime: "image/png",
        data: picture(rng.uint64(), width, height, transparent),
    };
    let portrait = png("demo-portrait.png", 240, 400, false);
    let landscape = png("demo-landscape.png", 640, 400, false);
    let square = png("demo-square.png", 360, 360, false);
    let panorama = png("demo-panorama.png", 1024, 256, false);
    let transparent = png("demo-transparent.png", 256, 256, true);
    vec![
        portrait,
        landscape,
        square,
        panorama,
        transparent,
        Asset {
            name: "demo-motion.gif",
            mime: "image/gif",
            data: animation(),
        },
        Asset {
            name: "demo-video.mp4",
            mime: "video/mp4",
            data: fixture("video"),
        },
        Asset {
            name: "demo-audio.m4a",
            mime: "audio/mp4",
            data: fixture("audio"),
        },
        Asset {
            name: "demo-field-notes.pdf",
            mime: "application/pdf",
            data: document(),
        },
        Asset {
            name: "demo-contact.vcf",
            mime: "text/vcard",
            data: b"BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Demo Aster Example\r\nN:Example;Demo Aster;;;\r\nTEL:+1 555 0100\r\nEMAIL:demo.person00@example.com\r\nEND:VCARD\r\n".to_vec(),
        },
    ]
}
