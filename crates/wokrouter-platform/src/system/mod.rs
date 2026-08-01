pub mod locale;
pub mod paths;
pub mod private_paths;
mod process;
#[cfg(windows)]
pub(crate) mod windows_security;
pub mod wokcore;

pub(super) fn process_executable_matches(
    process_id: std::num::NonZeroU32,
    candidate: &std::path::Path,
) -> bool {
    process::process_executable_matches(process_id, candidate)
}
