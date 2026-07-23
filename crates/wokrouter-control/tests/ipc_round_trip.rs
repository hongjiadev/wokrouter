use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::Notify,
};
use wokrouter_control::{
    CONTROL_PROTOCOL_VERSION, ControlClient, ControlEndpoint, ControlError, ControlRequest,
    ControlResponse, ControlServer, DaemonState, DaemonStatus,
};

async fn ping_handler(request: ControlRequest) -> ControlResponse {
    assert!(matches!(request, ControlRequest::Ping));
    ControlResponse::Pong {
        protocol_version: CONTROL_PROTOCOL_VERSION,
    }
}

#[tokio::test]
async fn ping_round_trip_negotiates_protocol_version() {
    let endpoint = ControlEndpoint::temporary("foundation-ping").unwrap();
    let server = ControlServer::bind(endpoint.clone(), ping_handler)
        .await
        .unwrap();
    let client = ControlClient::connect(&endpoint).await.unwrap();
    let response = client.request(ControlRequest::Ping).await.unwrap();

    assert_eq!(
        response,
        ControlResponse::Pong {
            protocol_version: 1
        }
    );

    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn daemon_status_round_trips_both_states() {
    for state in [DaemonState::Running, DaemonState::Stopped] {
        let expected = DaemonStatus {
            state,
            version: "0.1.0".to_owned(),
        };
        let endpoint = ControlEndpoint::temporary("foundation-status").unwrap();
        let response = expected.clone();
        let server = ControlServer::bind(endpoint.clone(), move |request| {
            let response = response.clone();
            async move {
                assert_eq!(request, ControlRequest::Status);
                ControlResponse::Status(response)
            }
        })
        .await
        .unwrap();
        let client = ControlClient::connect(&endpoint).await.unwrap();

        assert_eq!(
            client.request(ControlRequest::Status).await.unwrap(),
            ControlResponse::Status(expected)
        );

        server.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn oversized_prefix_is_rejected_before_reading_a_body() {
    let endpoint = ControlEndpoint::temporary("foundation-oversized").unwrap();
    let server = ControlServer::bind(endpoint.clone(), ping_handler)
        .await
        .unwrap();
    let mut stream = connect_raw(&endpoint).await.unwrap();

    stream.write_all(&u32::MAX.to_be_bytes()).await.unwrap();
    let mut byte = [0_u8; 1];
    let read = tokio::time::timeout(Duration::from_secs(2), stream.read(&mut byte))
        .await
        .expect("server did not reject the oversized prefix")
        .unwrap();

    assert_eq!(read, 0);

    let client = ControlClient::connect(&endpoint).await.unwrap();
    assert!(matches!(
        client.request(ControlRequest::Ping).await.unwrap(),
        ControlResponse::Pong { .. }
    ));
    server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn partial_frames_time_out_without_exhausting_connection_capacity() {
    let endpoint = ControlEndpoint::temporary("foundation-partial-timeout").unwrap();
    let server = ControlServer::bind(endpoint.clone(), ping_handler)
        .await
        .unwrap();
    let mut partial_clients = Vec::new();

    for _ in 0..64 {
        let mut stream = connect_raw(&endpoint).await.unwrap();
        stream.write_all(&64_u32.to_be_bytes()).await.unwrap();
        stream.write_all(b"{").await.unwrap();
        stream.flush().await.unwrap();
        partial_clients.push(stream);
    }
    tokio::time::sleep(Duration::from_millis(100)).await;

    let response = tokio::time::timeout(Duration::from_secs(3), async {
        let client = ControlClient::connect(&endpoint).await?;
        client.request(ControlRequest::Ping).await
    })
    .await
    .expect("partial frames prevented a healthy client from completing")
    .expect("server did not recover capacity from partial frames");

    assert!(matches!(response, ControlResponse::Pong { .. }));

    drop(partial_clients);
    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn incompatible_protocol_version_returns_typed_error_without_dispatch() {
    let endpoint = ControlEndpoint::temporary("foundation-version").unwrap();
    let dispatched = Arc::new(AtomicBool::new(false));
    let handler_dispatched = Arc::clone(&dispatched);
    let server = ControlServer::bind(endpoint.clone(), move |_| {
        handler_dispatched.store(true, Ordering::SeqCst);
        async {
            ControlResponse::Pong {
                protocol_version: CONTROL_PROTOCOL_VERSION,
            }
        }
    })
    .await
    .unwrap();
    let client =
        ControlClient::connect_with_protocol_version(&endpoint, CONTROL_PROTOCOL_VERSION + 1)
            .await
            .unwrap();

    let error = client.request(ControlRequest::Ping).await.unwrap_err();

    assert_eq!(
        error,
        ControlError::IncompatibleVersion {
            client: 2,
            daemon: 1,
        }
    );
    assert!(!dispatched.load(Ordering::SeqCst));
    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn cancelled_request_reconnects_before_the_next_transaction() {
    let endpoint = ControlEndpoint::temporary("foundation-cancelled-request").unwrap();
    let dispatch_count = Arc::new(AtomicUsize::new(0));
    let first_dispatched = Arc::new(Notify::new());
    let server = ControlServer::bind(endpoint.clone(), {
        let dispatch_count = Arc::clone(&dispatch_count);
        let first_dispatched = Arc::clone(&first_dispatched);
        move |request| {
            let call = dispatch_count.fetch_add(1, Ordering::SeqCst);
            let first_dispatched = Arc::clone(&first_dispatched);
            async move {
                assert_eq!(request, ControlRequest::Ping);
                if call == 0 {
                    first_dispatched.notify_one();
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
                ControlResponse::Pong {
                    protocol_version: CONTROL_PROTOCOL_VERSION,
                }
            }
        }
    })
    .await
    .unwrap();
    let client = Arc::new(ControlClient::connect(&endpoint).await.unwrap());
    let first_request = tokio::spawn({
        let client = Arc::clone(&client);
        async move { client.request(ControlRequest::Ping).await }
    });

    first_dispatched.notified().await;
    first_request.abort();
    assert!(first_request.await.unwrap_err().is_cancelled());

    let response = client.request(ControlRequest::Ping).await.unwrap();
    assert_eq!(
        response,
        ControlResponse::Pong {
            protocol_version: CONTROL_PROTOCOL_VERSION,
        }
    );
    assert_eq!(dispatch_count.load(Ordering::SeqCst), 2);

    server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_clients_are_served_independently() {
    let endpoint = ControlEndpoint::temporary("foundation-concurrent").unwrap();
    let server = ControlServer::bind(endpoint.clone(), ping_handler)
        .await
        .unwrap();
    let mut tasks = Vec::new();

    for _ in 0..32 {
        let endpoint = endpoint.clone();
        tasks.push(tokio::spawn(async move {
            let client = ControlClient::connect(&endpoint).await.unwrap();
            client.request(ControlRequest::Ping).await.unwrap()
        }));
    }

    for task in tasks {
        assert_eq!(
            task.await.unwrap(),
            ControlResponse::Pong {
                protocol_version: CONTROL_PROTOCOL_VERSION,
            }
        );
    }

    server.shutdown().await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn unix_endpoint_is_current_user_only() {
    use std::os::unix::fs::PermissionsExt;

    let endpoint = ControlEndpoint::temporary("foundation-permissions").unwrap();
    let server = ControlServer::bind(endpoint.clone(), ping_handler)
        .await
        .unwrap();
    let mode = std::fs::metadata(endpoint.as_path())
        .unwrap()
        .permissions()
        .mode()
        & 0o777;

    assert_eq!(mode, 0o600);
    server.shutdown().await.unwrap();
}

#[cfg(windows)]
#[tokio::test]
async fn windows_endpoint_is_current_user_only() {
    let endpoint = ControlEndpoint::temporary("foundation-permissions").unwrap();
    let server = ControlServer::bind(endpoint.clone(), ping_handler)
        .await
        .unwrap();
    let stream = connect_raw(&endpoint).await.unwrap();

    assert_pipe_dacl_is_current_user_only(&stream).unwrap();

    drop(stream);
    server.shutdown().await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn stale_unix_endpoint_is_removed_only_when_no_listener_accepts() {
    use tokio::net::UnixListener;

    let stale_endpoint = ControlEndpoint::temporary("foundation-stale").unwrap();
    let stale_listener = UnixListener::bind(stale_endpoint.as_path()).unwrap();
    drop(stale_listener);

    let server = ControlServer::bind(stale_endpoint.clone(), ping_handler)
        .await
        .unwrap();
    server.shutdown().await.unwrap();
    assert!(!stale_endpoint.as_path().exists());

    let live_endpoint = ControlEndpoint::temporary("foundation-live").unwrap();
    let live_listener = UnixListener::bind(live_endpoint.as_path()).unwrap();
    let error = ControlServer::bind(live_endpoint.clone(), ping_handler)
        .await
        .unwrap_err();

    assert!(matches!(error, ControlError::EndpointInUse));
    assert!(live_endpoint.as_path().exists());
    drop(live_listener);
    std::fs::remove_file(live_endpoint.as_path()).unwrap();
}

#[tokio::test]
async fn shutdown_closes_existing_clients_and_stops_accepting_connections() {
    let endpoint = ControlEndpoint::temporary("foundation-shutdown").unwrap();
    let server = ControlServer::bind(endpoint.clone(), ping_handler)
        .await
        .unwrap();
    let client = ControlClient::connect(&endpoint).await.unwrap();

    server.shutdown().await.unwrap();

    let request =
        tokio::time::timeout(Duration::from_secs(2), client.request(ControlRequest::Ping))
            .await
            .expect("existing connection remained open after shutdown");
    assert!(request.is_err());
    assert!(ControlClient::connect(&endpoint).await.is_err());
}

#[cfg(unix)]
async fn connect_raw(endpoint: &ControlEndpoint) -> io::Result<tokio::net::UnixStream> {
    tokio::net::UnixStream::connect(endpoint.as_path()).await
}

#[cfg(windows)]
async fn connect_raw(
    endpoint: &ControlEndpoint,
) -> io::Result<tokio::net::windows::named_pipe::NamedPipeClient> {
    use windows_sys::Win32::Foundation::ERROR_PIPE_BUSY;

    for _ in 0..100 {
        match tokio::net::windows::named_pipe::ClientOptions::new().open(endpoint.as_pipe_name()) {
            Ok(stream) => return Ok(stream),
            Err(error) if error.raw_os_error() == Some(ERROR_PIPE_BUSY as i32) => {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::from_raw_os_error(ERROR_PIPE_BUSY as i32))
}

#[cfg(windows)]
fn assert_pipe_dacl_is_current_user_only(
    pipe: &tokio::net::windows::named_pipe::NamedPipeClient,
) -> io::Result<()> {
    use std::{ffi::c_void, mem::size_of, os::windows::io::AsRawHandle, ptr};

    use windows_sys::Win32::{
        Foundation::{CloseHandle, ERROR_SUCCESS, HANDLE, LocalFree},
        Security::{
            ACCESS_ALLOWED_ACE, ACE_HEADER, ACL, ACL_SIZE_INFORMATION, AclSizeInformation,
            Authorization::{GetSecurityInfo, SE_KERNEL_OBJECT},
            DACL_SECURITY_INFORMATION, EqualSid, GetAce, GetAclInformation, GetTokenInformation,
            OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, TOKEN_QUERY, TOKEN_USER,
            TokenUser,
        },
        System::{
            SystemServices::ACCESS_ALLOWED_ACE_TYPE,
            Threading::{GetCurrentProcess, OpenProcessToken},
        },
    };

    struct LocalDescriptor(PSECURITY_DESCRIPTOR);
    impl Drop for LocalDescriptor {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { LocalFree(self.0.cast()) };
            }
        }
    }

    struct Token(HANDLE);
    impl Drop for Token {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.0) };
        }
    }

    let mut owner: PSID = ptr::null_mut();
    let mut dacl: *mut ACL = ptr::null_mut();
    let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
    let status = unsafe {
        GetSecurityInfo(
            pipe.as_raw_handle(),
            SE_KERNEL_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            ptr::null_mut(),
            &mut dacl,
            ptr::null_mut(),
            &mut descriptor,
        )
    };
    let descriptor = LocalDescriptor(descriptor);
    if status != ERROR_SUCCESS {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    if owner.is_null() || dacl.is_null() {
        return Err(io::Error::other("pipe has no owner or DACL"));
    }

    let mut token_handle: HANDLE = ptr::null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token_handle) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let token = Token(token_handle);
    let mut required = 0;
    unsafe { GetTokenInformation(token.0, TokenUser, ptr::null_mut(), 0, &mut required) };
    if required == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut user_buffer = vec![0_usize; (required as usize).div_ceil(size_of::<usize>())];
    if unsafe {
        GetTokenInformation(
            token.0,
            TokenUser,
            user_buffer.as_mut_ptr().cast::<c_void>(),
            required,
            &mut required,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let token_user = unsafe { &*(user_buffer.as_ptr().cast::<TOKEN_USER>()) };
    if unsafe { EqualSid(owner, token_user.User.Sid) } == 0 {
        return Err(io::Error::other("pipe owner is not the current user"));
    }

    let mut info = ACL_SIZE_INFORMATION::default();
    if unsafe {
        GetAclInformation(
            dacl,
            (&mut info as *mut ACL_SIZE_INFORMATION).cast::<c_void>(),
            size_of::<ACL_SIZE_INFORMATION>() as u32,
            AclSizeInformation,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    if info.AceCount == 0 {
        return Err(io::Error::other("pipe DACL has no allow entry"));
    }

    let mut allow_count = 0;
    for index in 0..info.AceCount {
        let mut ace: *mut c_void = ptr::null_mut();
        if unsafe { GetAce(dacl, index, &mut ace) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let header = unsafe { &*ace.cast::<ACE_HEADER>() };
        if header.AceType as u32 != ACCESS_ALLOWED_ACE_TYPE {
            continue;
        }
        allow_count += 1;
        let allowed = unsafe { &*ace.cast::<ACCESS_ALLOWED_ACE>() };
        let sid = (&raw const allowed.SidStart).cast_mut().cast::<c_void>();
        if unsafe { EqualSid(sid, token_user.User.Sid) } == 0 {
            return Err(io::Error::other("pipe grants access to another principal"));
        }
    }
    if allow_count == 0 {
        return Err(io::Error::other("pipe DACL has no allow entry"));
    }

    drop(token);
    drop(descriptor);
    Ok(())
}
