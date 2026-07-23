#[test]
fn workspace_exposes_build_identity() {
    let info = wokrouter_core::build::BuildInfo::current();
    assert_eq!(info.product, "WokRouter");
    assert_eq!(info.version, env!("CARGO_PKG_VERSION"));
    assert_eq!(info.control_protocol, 1);
}
