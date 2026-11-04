use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, fs, path::Path};

const MANIFEST: &str = include_str!("../assets/SHA256SUMS");

fn files(directory: &Path, prefix: &str, out: &mut BTreeSet<String>) {
    for entry in fs::read_dir(directory).unwrap_or_else(|_| panic!("{}", directory.display())) {
        let entry = entry.unwrap_or_else(|_| panic!("entry"));
        let name = format!("{prefix}{}", entry.file_name().to_string_lossy());
        if entry.path().is_dir() {
            files(&entry.path(), &format!("{name}/"), out);
        } else {
            out.insert(name);
        }
    }
}

#[test]
fn frozen_media_assets_match_their_recorded_digests() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
    let mut listed = BTreeSet::new();
    for line in MANIFEST.lines() {
        let (digest, name) = line.split_once("  ").unwrap_or_else(|| panic!("bad line {line}"));
        let bytes = fs::read(root.join(name)).unwrap_or_else(|_| panic!("missing {name}"));
        assert_eq!(hex::encode(Sha256::digest(&bytes)), digest, "{name} changed");
        listed.insert(name.to_owned());
    }
    let mut present = BTreeSet::new();
    files(&root, "", &mut present);
    present.remove("SHA256SUMS");
    present.remove("README.md");
    assert_eq!(present, listed, "every asset must be listed in SHA256SUMS");
}

#[test]
fn recorded_assets_are_the_embedded_fixture_bytes() {
    let manifest: std::collections::BTreeMap<&str, &str> = MANIFEST
        .lines()
        .filter_map(|line| line.split_once("  "))
        .map(|(digest, name)| (name, digest))
        .collect();
    for (name, bytes) in [
        ("video-default.mp4", include_bytes!("../assets/video-default.mp4").as_slice()),
        ("video-landscape.mp4", include_bytes!("../assets/video-landscape.mp4").as_slice()),
        ("video-portrait.mp4", include_bytes!("../assets/video-portrait.mp4").as_slice()),
        ("video-panorama.mp4", include_bytes!("../assets/video-panorama.mp4").as_slice()),
        ("video-live.mp4", include_bytes!("../assets/video-live.mp4").as_slice()),
        ("live-still.png", include_bytes!("../assets/live-still.png").as_slice()),
        ("audio.m4a", include_bytes!("../assets/audio.m4a").as_slice()),
    ] {
        assert_eq!(manifest.get(name).copied(), Some(hex::encode(Sha256::digest(bytes)).as_str()), "{name}");
    }
}
