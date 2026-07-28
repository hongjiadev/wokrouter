use std::path::PathBuf;

use wokrouter_wokcore_client::{AuthorizationError, AuthorizationState, WokCoreAuthorizer};

#[tokio::test]
async fn unavailable_authorizer_exposes_neither_path_nor_secret_material() {
    let executable = PathBuf::from(r"Z:\missing-private-path\wokcore.exe");
    let authorizer = WokCoreAuthorizer::new(&executable);

    let error = authorizer.authorize().await.unwrap_err();

    assert_eq!(error, AuthorizationError::Unavailable);
    assert!(!format!("{authorizer:?}").contains("missing-private-path"));
    assert!(!format!("{error:?}").contains("missing-private-path"));
    assert!(!error.to_string().contains("missing-private-path"));
}

#[test]
fn authorization_state_is_a_secret_free_frontend_contract() {
    assert_eq!(format!("{:?}", AuthorizationState::Ready), "Ready");
    assert_eq!(format!("{:?}", AuthorizationState::Required), "Required");
    assert_eq!(
        format!("{:?}", AuthorizationState::Unsupported),
        "Unsupported"
    );
}
