use std::path::{Path, PathBuf};

const PLIST: &str = "Contents/Library/LaunchAgents/dev.applebridge.gateway.plist";

#[must_use]
pub fn detect(executable: &Path) -> Option<PathBuf> {
    let bundle = executable.parent()?.parent()?.parent()?;
    bundle.join(PLIST).exists().then(|| bundle.to_path_buf())
}

#[must_use]
pub fn helper_app(explicit: &str, executable: Option<&Path>) -> String {
    if !explicit.is_empty() {
        return explicit.to_owned();
    }
    executable
        .and_then(detect)
        .map_or_else(String::new, |bundle| bundle.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn packaged(root: &Path) -> PathBuf {
        let plist = root.join("Apple Bridge.app").join(PLIST);
        std::fs::create_dir_all(plist.parent().unwrap()).unwrap();
        std::fs::write(&plist, b"plist").unwrap();
        root.join("Apple Bridge.app/Contents/MacOS/bridge")
    }
    #[test]
    fn packaged_executable_resolves_its_bundle() {
        let root = tempfile::tempdir().unwrap();
        let executable = packaged(root.path());
        assert_eq!(
            detect(&executable),
            Some(root.path().join("Apple Bridge.app"))
        );
        assert_eq!(
            helper_app("", Some(&executable)),
            root.path().join("Apple Bridge.app").to_string_lossy()
        );
    }
    #[test]
    fn explicit_flag_wins_and_unpackaged_binaries_have_no_helper() {
        let root = tempfile::tempdir().unwrap();
        let executable = packaged(root.path());
        assert_eq!(helper_app("/custom/Helper.app", Some(&executable)), "/custom/Helper.app");
        assert_eq!(helper_app("", Some(&root.path().join("a/b/bridge"))), "");
        assert_eq!(helper_app("", Some(Path::new("bridge"))), "");
        assert_eq!(helper_app("", None), "");
    }
}
