use std::{ffi::OsStr, io};

use uuid::Uuid;

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
pub(crate) use unix::{ClientStream, Listener, ServerStream, bind, connect};
#[cfg(windows)]
pub(crate) use windows::{ClientStream, Listener, ServerStream, bind, connect};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ControlEndpoint {
    #[cfg(windows)]
    pipe_name: String,
    #[cfg(unix)]
    path: std::path::PathBuf,
}

impl ControlEndpoint {
    pub fn temporary(label: &str) -> io::Result<Self> {
        if label.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "control endpoint label must not be empty",
            ));
        }
        let label: String = label
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                    character
                } else {
                    '-'
                }
            })
            .collect();
        let name = format!("wokrouter-{label}-{}", Uuid::new_v4());

        #[cfg(windows)]
        {
            Ok(Self {
                pipe_name: format!(r"\\.\pipe\{name}"),
            })
        }
        #[cfg(unix)]
        {
            Ok(Self {
                path: std::env::temp_dir().join(format!("{name}.sock")),
            })
        }
    }

    #[cfg(windows)]
    pub fn as_pipe_name(&self) -> &OsStr {
        OsStr::new(&self.pipe_name)
    }

    #[cfg(unix)]
    pub fn as_path(&self) -> &std::path::Path {
        &self.path
    }

    pub(crate) fn cleanup(&self) {
        #[cfg(unix)]
        {
            match std::fs::remove_file(&self.path) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(_) => {}
            }
        }
    }
}

impl AsRef<OsStr> for ControlEndpoint {
    fn as_ref(&self) -> &OsStr {
        #[cfg(windows)]
        {
            self.as_pipe_name()
        }
        #[cfg(unix)]
        {
            self.as_path().as_os_str()
        }
    }
}
