#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct InFocusEvent {
    pub time: f64,
    pub starting: bool,
    pub bundle: String,
}

pub const VERSION: &str = "infocus-segb-1";

pub fn decode(data: &[u8]) -> Vec<InFocusEvent> {
    let mut events = Vec::new();
    for bytes in data.windows(16).enumerate() {
        let (index, header) = bytes;
        if header[0] != 0x10
            || header[2] != 0x18
            || header[3] > 1
            || header[4] != 0x21
            || header[13] != 0x32
        {
            continue;
        }
        let length = usize::from(header[14]);
        let Some(bundle) = data.get(index + 15..index + 15 + length) else {
            continue;
        };
        if bundle.is_empty()
            || !bundle
                .iter()
                .all(|b| b.is_ascii_alphanumeric() || b"-._".contains(b))
        {
            continue;
        }
        let mut bits = [0; 8];
        bits.copy_from_slice(&header[5..13]);
        let time = f64::from_le_bytes(bits) + 978_307_200.0;
        if time.is_finite() && time > 1.6e9 && time < 2.1e9 {
            events.push(InFocusEvent {
                time,
                starting: header[3] == 1,
                bundle: String::from_utf8_lossy(bundle).into_owned(),
            });
        }
    }
    events
}
