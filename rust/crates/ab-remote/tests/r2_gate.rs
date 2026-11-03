use ab_remote::{ct, state};

#[test]
fn r2_requires_build_and_local_opt_in() -> Result<(), Box<dyn std::error::Error>> {
    let home = tempfile::tempdir()?;
    assert!(!ct::enabled(home.path()));
    state::write(home.path(), "r2-certificates", &true)?;
    assert_eq!(ct::enabled(home.path()), cfg!(feature = "r2-certificates"));
    state::write(home.path(), "r2-certificates", &false)?;
    assert!(!ct::enabled(home.path()));
    Ok(())
}
