use zune_core::{
    bytestream::ZCursor, colorspace::ColorSpace, options::DecoderOptions, result::DecodingResult,
};

#[derive(Debug)]
pub struct Image {
    pub width: usize,
    pub height: usize,
    channels: usize,
    pixels: Vec<u8>,
}
impl Image {
    #[must_use]
    pub fn decode(data: &[u8]) -> Option<Self> {
        if data.starts_with(b"\x89PNG\r\n\x1a\n") {
            let options = DecoderOptions::default().png_set_strip_to_8bit(true);
            let mut decoder = zune_png::PngDecoder::new_with_options(ZCursor::new(data), options);
            let DecodingResult::U8(pixels) = decoder.decode().ok()? else {
                return None;
            };
            let (width, height) = decoder.dimensions()?;
            let channels = decoder.colorspace()?.num_components();
            return Self::checked(width, height, channels, pixels);
        }
        if data.starts_with(&[0xff, 0xd8, 0xff]) {
            let options = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGB);
            let mut decoder = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(data), options);
            let pixels = decoder.decode().ok()?;
            let info = decoder.info()?;
            return Self::checked(usize::from(info.width), usize::from(info.height), 3, pixels);
        }
        None
    }
    fn checked(width: usize, height: usize, channels: usize, pixels: Vec<u8>) -> Option<Self> {
        (channels >= 3 && pixels.len() >= width * height * channels).then_some(Self {
            width,
            height,
            channels,
            pixels,
        })
    }
    #[must_use]
    pub fn rgb16(&self, x: usize, y: usize) -> [f64; 3] {
        let at = (y * self.width + x) * self.channels;
        [0, 1, 2].map(|i| f64::from(self.pixels[at + i]) * 257.0)
    }
    #[must_use]
    pub fn center_marker(&self) -> bool {
        let [r, g, b] = self.rgb16(self.width / 2, self.height / 2);
        r > 50000.0 && g < 12000.0 && b < 12000.0
    }
    #[must_use]
    pub fn edge_contrast(&self) -> f64 {
        let height = i64::try_from(self.height).unwrap_or(0);
        let width = i64::try_from(self.width).unwrap_or(0);
        let scale = f64::from(u32::try_from(height).unwrap_or(0)) / 360.0;
        let edge = f64::from(u32::try_from(width).unwrap_or(0)) / 2.0
            - f64::from(u32::try_from(height).unwrap_or(0)) * 0.4;
        let sample = |x: f64, y: usize| -> f64 {
            let x = x - 0.5;
            let floor = x.floor();
            let left = clamp(floor, width);
            let right = (left + 1).min(width - 1);
            let fraction = x - floor;
            let l = self.rgb16(usize::try_from(left).unwrap_or(0), y);
            let r = self.rgb16(usize::try_from(right).unwrap_or(0), y);
            (l[0] - l[2]) * (1.0 - fraction) + (r[0] - r[2]) * fraction
        };
        let mut contrast = 0.0_f64;
        for offset in -3_i32..=3 {
            let mut delta = 0.0;
            for row in -2_i64..=2 {
                let y = usize::try_from((height / 2 + row).clamp(0, height - 1)).unwrap_or(0);
                let center = edge + f64::from(offset) * scale;
                delta +=
                    (sample(center + 1.5 * scale, y) - sample(center - 1.5 * scale, y)) / 131_070.0;
            }
            contrast = contrast.max(delta / 5.0);
        }
        contrast
    }
}
#[allow(clippy::cast_possible_truncation)]
fn clamp(value: f64, width: i64) -> i64 {
    (value as i64).clamp(0, width - 1)
}
