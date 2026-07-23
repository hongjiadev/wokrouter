use std::{future::Future, sync::Arc};

use tokio::{sync::watch, task::JoinSet};

use crate::{
    CONTROL_PROTOCOL_VERSION, ControlEndpoint, ControlError, ControlRequest, ControlResponse,
    codec::{read_frame, write_frame},
    protocol::Frame,
    transport::{ServerStream, bind},
};

const MAX_CONNECTION_TASKS: usize = 64;

#[derive(Debug)]
pub struct ControlServer {
    shutdown: watch::Sender<bool>,
    task: Option<tokio::task::JoinHandle<Result<(), ControlError>>>,
}

impl ControlServer {
    pub async fn bind<H, F>(endpoint: ControlEndpoint, handler: H) -> Result<Self, ControlError>
    where
        H: Fn(ControlRequest) -> F + Send + Sync + 'static,
        F: Future<Output = ControlResponse> + Send + 'static,
    {
        let listener = bind(&endpoint).await?;
        let (shutdown, receiver) = watch::channel(false);
        let handler = Arc::new(handler);
        let task = tokio::spawn(run_server(listener, endpoint, handler, receiver));
        Ok(Self {
            shutdown,
            task: Some(task),
        })
    }

    pub async fn shutdown(mut self) -> Result<(), ControlError> {
        let _ = self.shutdown.send(true);
        let Some(task) = self.task.take() else {
            return Ok(());
        };
        task.await.map_err(|_| ControlError::ServerTaskFailed)?
    }
}

impl Drop for ControlServer {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
    }
}

async fn run_server<H, F>(
    mut listener: crate::transport::Listener,
    endpoint: ControlEndpoint,
    handler: Arc<H>,
    mut shutdown: watch::Receiver<bool>,
) -> Result<(), ControlError>
where
    H: Fn(ControlRequest) -> F + Send + Sync + 'static,
    F: Future<Output = ControlResponse> + Send + 'static,
{
    let mut connections = JoinSet::new();
    let result = loop {
        tokio::select! {
            biased;
            changed = shutdown.changed() => {
                let _ = changed;
                break Ok(());
            }
            accepted = listener.accept(), if has_connection_capacity(connections.len()) => {
                match accepted {
                    Ok(stream) => {
                        connections.spawn(serve_connection(
                            stream,
                            Arc::clone(&handler),
                            shutdown.clone(),
                        ));
                    }
                    Err(error) => break Err(error.into()),
                }
            }
            Some(_) = connections.join_next(), if !connections.is_empty() => {}
        }
    };

    connections.abort_all();
    while connections.join_next().await.is_some() {}
    drop(listener);
    endpoint.cleanup();
    result
}

fn has_connection_capacity(active: usize) -> bool {
    active < MAX_CONNECTION_TASKS
}

async fn serve_connection<H, F>(
    mut stream: ServerStream,
    handler: Arc<H>,
    mut shutdown: watch::Receiver<bool>,
) where
    H: Fn(ControlRequest) -> F + Send + Sync + 'static,
    F: Future<Output = ControlResponse> + Send + 'static,
{
    loop {
        let request: Frame<ControlRequest> = tokio::select! {
            biased;
            _ = shutdown.changed() => return,
            request = read_frame(&mut stream) => match request {
                Ok(request) => request,
                Err(_) => return,
            },
        };

        if request.protocol_version != CONTROL_PROTOCOL_VERSION {
            let response = Frame {
                protocol_version: CONTROL_PROTOCOL_VERSION,
                request_id: request.request_id,
                payload: ControlResponse::Error(ControlError::IncompatibleVersion {
                    client: request.protocol_version,
                    daemon: CONTROL_PROTOCOL_VERSION,
                }),
            };
            let _ = write_frame(&mut stream, &response).await;
            return;
        }

        let response = tokio::select! {
            biased;
            _ = shutdown.changed() => return,
            response = handler(request.payload) => response,
        };
        let response = Frame {
            protocol_version: CONTROL_PROTOCOL_VERSION,
            request_id: request.request_id,
            payload: response,
        };
        if write_frame(&mut stream, &response).await.is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::has_connection_capacity;

    #[test]
    fn connection_admission_stops_at_the_task_limit() {
        assert!(has_connection_capacity(63));
        assert!(!has_connection_capacity(64));
    }
}
