use plist::{Dictionary, Uid, Value};

pub fn archive(text: &str, segments: usize) -> Vec<u8> {
    let mut segment = Dictionary::new();
    segment.insert("text".into(), Value::Uid(Uid::new(3)));
    let mut array = Dictionary::new();
    array.insert(
        "NS.objects".into(),
        Value::Array(vec![Value::Uid(Uid::new(2)); segments]),
    );
    let mut top = Dictionary::new();
    top.insert("root".into(), Value::Uid(Uid::new(1)));
    let mut root = Dictionary::new();
    root.insert(
        "$objects".into(),
        Value::Array(vec![
            Value::String("$null".into()),
            Value::Dictionary(array),
            Value::Dictionary(segment),
            Value::String(text.into()),
        ]),
    );
    root.insert("$top".into(), Value::Dictionary(top));
    let mut bytes = Vec::new();
    Value::Dictionary(root)
        .to_writer_binary(&mut bytes)
        .unwrap();
    bytes
}

pub fn repeated_strings(text: &str, count: usize) -> Vec<u8> {
    let value = Value::Array(vec![Value::String(text.into()); count]);
    let mut bytes = Vec::new();
    value.to_writer_binary(&mut bytes).unwrap();
    bytes
}
