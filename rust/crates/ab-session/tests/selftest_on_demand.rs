#![allow(clippy::expect_used)]
#[test]
fn the_messages_self_test_runs_only_when_asked() {
    let helper = include_str!("../../../../native/Sources/AppleSession/HelperCommand.swift");
    let starts = helper.matches("RustSelfTest.shared.run(").count();
    assert_eq!(starts, 1, "only the selftest command may start it");
    let asked = helper
        .find("dropFirst().first == \"messages\"")
        .expect("selftest command");
    let start = helper.find("RustSelfTest.shared.run(").expect("start");
    assert!(start > asked, "the self-test starts before anyone asked");
}
