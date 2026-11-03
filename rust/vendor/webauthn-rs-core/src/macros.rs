macro_rules! cbor_try_map {
    (
        $v:expr
    ) => {{
        match $v {
            serde_cbor_2::Value::Map(m) => Ok(m),
            _ => Err(WebauthnError::COSEKeyInvalidCBORValue),
        }
    }};
}

macro_rules! cbor_try_array {
    (
        $v:expr
    ) => {{
        match $v {
            serde_cbor_2::Value::Array(m) => Ok(m),
            _ => Err(WebauthnError::COSEKeyInvalidCBORValue),
        }
    }};
}

macro_rules! cbor_try_string {
    (
        $v:expr
    ) => {{
        match $v {
            serde_cbor_2::Value::Text(m) => Ok(m),
            _ => Err(WebauthnError::COSEKeyInvalidCBORValue),
        }
    }};
}

macro_rules! cbor_try_bytes {
    (
        $v:expr
    ) => {{
        match $v {
            serde_cbor_2::Value::Bytes(m) => Ok(m),
            _ => Err(WebauthnError::COSEKeyInvalidCBORValue),
        }
    }};
}

macro_rules! cbor_try_i128 {
    (
        $v:expr
    ) => {{
        match $v {
            serde_cbor_2::Value::Integer(m) => Ok(*m),
            _ => Err(WebauthnError::COSEKeyInvalidCBORValue),
        }
    }};
}
































