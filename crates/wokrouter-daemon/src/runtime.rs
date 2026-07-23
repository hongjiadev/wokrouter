use std::{
    fs, io,
    net::{IpAddr, Ipv4Addr},
    path::{Path, PathBuf},
    sync::Arc,
};

use tokio::{
    net::TcpListener,
    sync::{Mutex, watch},
};
use wokrouter_control::{
    CONTROL_PROTOCOL_VERSION, ControlEndpoint, ControlError, ControlRequest, ControlResponse,
    ControlServer, DaemonState, DaemonStatus,
};
use wokrouter_platform::AppPaths;
use wokrouter_storage::{AppConfig, ConfigStore, StorageError};

const DEFAULT_DATA_PLANE_PORT: u16 = 10101;
const PID_FILE_NAME: &str = "wokrouterd.pid";

pub struct DaemonRuntime;

impl DaemonRuntime {
    pub async fn start(paths: AppPaths) -> Result<RunningDaemon, DaemonError> {
        prepare_directories(&paths)?;
        let endpoint = ControlEndpoint::for_runtime_dir(&paths.runtime_dir)?;
        let store = ConfigStore::new(&paths.config_file);
        let (ready, _) = watch::channel(false);
        let (shutdown, shutdown_receiver) = watch::channel(false);
        let state = Arc::new(RuntimeState {
            store,
            data: Mutex::new(None),
            ready,
            shutdown,
            #[cfg(test)]
            reload_loaded: None,
        });
        let handler_state = Arc::clone(&state);
        let after_send_state = Arc::clone(&state);
        let server = ControlServer::bind_with_after_send(
            endpoint,
            move |request| {
                let state = Arc::clone(&handler_state);
                async move { state.handle(request).await }
            },
            move |request, response| after_send_state.after_response_sent(request, response),
        )
        .await?;

        if let Err(error) = state.initialize(&paths).await {
            let _ = server.shutdown().await;
            return Err(error);
        }

        let pid_file = paths.runtime_dir.join(PID_FILE_NAME);
        write_pid_file(&pid_file)?;
        state.ready.send_replace(true);

        Ok(RunningDaemon {
            server: Some(server),
            _state: state,
            shutdown_receiver,
            pid_file,
        })
    }
}

pub struct RunningDaemon {
    server: Option<ControlServer>,
    _state: Arc<RuntimeState>,
    shutdown_receiver: watch::Receiver<bool>,
    pid_file: PathBuf,
}

impl RunningDaemon {
    pub async fn run_until_shutdown(mut self) -> Result<(), DaemonError> {
        while !*self.shutdown_receiver.borrow() {
            self.shutdown_receiver
                .changed()
                .await
                .map_err(|_| DaemonError::ShutdownChannelClosed)?;
        }

        remove_owned_pid_file(&self.pid_file)?;
        if let Some(server) = self.server.take() {
            server.shutdown().await?;
        }
        Ok(())
    }
}

impl Drop for RunningDaemon {
    fn drop(&mut self) {
        let _ = remove_owned_pid_file(&self.pid_file);
    }
}

struct RuntimeState {
    store: ConfigStore,
    data: Mutex<Option<RuntimeData>>,
    ready: watch::Sender<bool>,
    shutdown: watch::Sender<bool>,
    #[cfg(test)]
    reload_loaded: Option<Arc<dyn Fn(u64) + Send + Sync>>,
}

struct RuntimeData {
    config: AppConfig,
    revision: u64,
    _data_plane: TcpListener,
}

impl RuntimeState {
    async fn initialize(&self, paths: &AppPaths) -> Result<(), DaemonError> {
        let config_exists = paths.config_file.try_exists()?;
        let mut loaded = self.store.load()?;
        let listener = match bind_data_plane(&loaded.config).await {
            Ok(listener) => listener,
            Err(error)
                if !config_exists
                    && error.kind() == io::ErrorKind::AddrInUse
                    && loaded.config.server.host == IpAddr::V4(Ipv4Addr::LOCALHOST)
                    && loaded.config.server.port == DEFAULT_DATA_PLANE_PORT =>
            {
                let listener = bind_fallback(&loaded.config).await?;
                let port = listener.local_addr()?.port();
                let mut candidate = loaded.config.clone();
                candidate.server.port = port;
                loaded = self.store.commit(loaded.revision, &candidate)?;
                listener
            }
            Err(error) if error.kind() == io::ErrorKind::AddrInUse => {
                return Err(DaemonError::ConfiguredPortInUse {
                    port: loaded.config.server.port,
                });
            }
            Err(source) => return Err(DaemonError::Io { source }),
        };

        *self.data.lock().await = Some(RuntimeData {
            config: loaded.config,
            revision: loaded.revision,
            _data_plane: listener,
        });
        Ok(())
    }

    async fn handle(&self, request: ControlRequest) -> ControlResponse {
        self.wait_until_ready().await;
        match request {
            ControlRequest::Ping => ControlResponse::Pong {
                protocol_version: CONTROL_PROTOCOL_VERSION,
            },
            ControlRequest::Status => ControlResponse::Status(DaemonStatus {
                state: DaemonState::Running,
                version: env!("CARGO_PKG_VERSION").to_owned(),
            }),
            ControlRequest::Reload { expected_revision } => self.reload(expected_revision).await,
            ControlRequest::Shutdown => ControlResponse::Accepted {
                revision: self.active_revision().await,
            },
        }
    }

    async fn wait_until_ready(&self) {
        let mut receiver = self.ready.subscribe();
        while !*receiver.borrow() {
            if receiver.changed().await.is_err() {
                return;
            }
        }
    }

    fn after_response_sent(&self, request: &ControlRequest, response: &ControlResponse) {
        if matches!(
            (request, response),
            (ControlRequest::Shutdown, ControlResponse::Accepted { .. })
        ) {
            let _ = self.shutdown.send(true);
        }
    }

    async fn reload(&self, expected_revision: u64) -> ControlResponse {
        let loaded = match self.store.load() {
            Ok(loaded) => loaded,
            Err(error) => {
                return ControlResponse::Error(ControlError::InvalidFrame {
                    message: error.to_string(),
                });
            }
        };
        if loaded.revision != expected_revision {
            return ControlResponse::Error(ControlError::RevisionConflict {
                expected: expected_revision,
                actual: loaded.revision,
            });
        }
        #[cfg(test)]
        if let Some(observer) = &self.reload_loaded {
            observer(loaded.revision);
        }

        let mut data = self.data.lock().await;
        let active = data
            .as_mut()
            .expect("runtime data must be initialized before requests are handled");
        if expected_revision < active.revision {
            return ControlResponse::Error(ControlError::RevisionConflict {
                expected: expected_revision,
                actual: active.revision,
            });
        }
        if active.config.server.host != loaded.config.server.host
            || active.config.server.port != loaded.config.server.port
        {
            let listener = match bind_data_plane(&loaded.config).await {
                Ok(listener) => listener,
                Err(error) if error.kind() == io::ErrorKind::AddrInUse => {
                    return ControlResponse::Error(ControlError::DataPlanePortInUse {
                        port: loaded.config.server.port,
                    });
                }
                Err(error) => {
                    return ControlResponse::Error(ControlError::Transport {
                        message: error.to_string(),
                    });
                }
            };
            active._data_plane = listener;
        }
        active.config = loaded.config;
        active.revision = loaded.revision;
        ControlResponse::Accepted {
            revision: active.revision,
        }
    }

    async fn active_revision(&self) -> u64 {
        self.data
            .lock()
            .await
            .as_ref()
            .expect("runtime data must be initialized before requests are handled")
            .revision
    }
}

async fn bind_data_plane(config: &AppConfig) -> io::Result<TcpListener> {
    TcpListener::bind((config.server.host, config.server.port)).await
}

async fn bind_fallback(config: &AppConfig) -> io::Result<TcpListener> {
    TcpListener::bind((config.server.host, 0)).await
}

fn prepare_directories(paths: &AppPaths) -> Result<(), DaemonError> {
    if let Some(config_dir) = paths.config_file.parent() {
        fs::create_dir_all(config_dir)?;
    }
    fs::create_dir_all(&paths.runtime_dir)?;
    fs::create_dir_all(&paths.log_dir)?;
    Ok(())
}

fn write_pid_file(path: &Path) -> Result<(), DaemonError> {
    fs::write(path, format!("{}\n", std::process::id()))?;
    Ok(())
}

fn remove_owned_pid_file(path: &Path) -> Result<(), DaemonError> {
    let expected = std::process::id().to_string();
    match fs::read_to_string(path) {
        Ok(contents) if contents.trim() == expected => fs::remove_file(path)?,
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(source) => return Err(DaemonError::Io { source }),
    }
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum DaemonError {
    #[error(transparent)]
    Control(#[from] ControlError),
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error("daemon I/O failed: {source}")]
    Io {
        #[from]
        source: io::Error,
    },
    #[error("configured data-plane port {port} is already in use")]
    ConfiguredPortInUse { port: u16 },
    #[error("daemon shutdown channel closed unexpectedly")]
    ShutdownChannelClosed,
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;
    use std::sync::{Arc, Barrier, mpsc};
    use std::time::Duration;

    use tokio::sync::{Mutex, watch};
    use wokrouter_control::{ControlRequest, ControlResponse};
    use wokrouter_storage::{AppConfig, ConfigStore};

    use super::{RuntimeData, RuntimeState};

    #[tokio::test]
    async fn shutdown_is_not_published_by_the_pre_flush_handler() {
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let (ready, _) = watch::channel(true);
        let (shutdown, shutdown_receiver) = watch::channel(false);
        let state = RuntimeState {
            store: ConfigStore::new("unused-config.toml"),
            data: Mutex::new(Some(RuntimeData {
                config: AppConfig::default(),
                revision: 7,
                _data_plane: listener,
            })),
            ready,
            shutdown,
            reload_loaded: None,
        };

        assert_eq!(
            state.handle(ControlRequest::Shutdown).await,
            ControlResponse::Accepted { revision: 7 }
        );
        assert!(
            !*shutdown_receiver.borrow(),
            "shutdown must remain unpublished until the Accepted response is flushed"
        );
        state.after_response_sent(
            &ControlRequest::Shutdown,
            &ControlResponse::Accepted { revision: 7 },
        );
        assert!(*shutdown_receiver.borrow());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stale_concurrent_reload_cannot_roll_back_the_active_revision() {
        let directory = tempfile::tempdir().unwrap();
        let store = ConfigStore::new(directory.path().join("config.toml"));
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let mut config = AppConfig::default();
        config.server.port = listener.local_addr().unwrap().port();
        let first = store.commit(0, &config).unwrap();
        let (ready, _) = watch::channel(true);
        let (shutdown, _) = watch::channel(false);
        let (loaded_sender, loaded_receiver) = mpsc::sync_channel(1);
        let release_old = Arc::new(Barrier::new(2));
        let observer_release = Arc::clone(&release_old);
        let first_revision = first.revision;
        let state = Arc::new(RuntimeState {
            store: store.clone(),
            data: Mutex::new(Some(RuntimeData {
                config: first.config.clone(),
                revision: first.revision,
                _data_plane: listener,
            })),
            ready,
            shutdown,
            reload_loaded: Some(Arc::new(move |revision| {
                if revision == first_revision {
                    loaded_sender.send(()).unwrap();
                    observer_release.wait();
                }
            })),
        });

        let old_state = Arc::clone(&state);
        let old_reload = tokio::spawn(async move { old_state.reload(first.revision).await });
        tokio::task::spawn_blocking(move || {
            loaded_receiver
                .recv_timeout(Duration::from_secs(2))
                .unwrap();
        })
        .await
        .unwrap();

        let second = store.commit(first.revision, &first.config).unwrap();
        assert_eq!(
            state.reload(second.revision).await,
            ControlResponse::Accepted {
                revision: second.revision,
            }
        );
        tokio::task::spawn_blocking(move || release_old.wait())
            .await
            .unwrap();

        assert_eq!(
            old_reload.await.unwrap(),
            ControlResponse::Error(wokrouter_control::ControlError::RevisionConflict {
                expected: first.revision,
                actual: second.revision,
            })
        );
        assert_eq!(
            state.data.lock().await.as_ref().unwrap().revision,
            second.revision
        );
    }
}
