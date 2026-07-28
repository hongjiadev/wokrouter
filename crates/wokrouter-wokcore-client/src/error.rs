#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ClientError {
    #[error("failed to initialize the WokCore HTTP client")]
    Initialization,
}
