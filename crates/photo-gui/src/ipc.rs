use std::io::{self, Read, Write};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
    mpsc::{self, Receiver, Sender, SyncSender},
};
use std::time::{Duration, Instant};

use interprocess::local_socket::{GenericNamespaced, ListenerOptions, Stream, prelude::*};
use photo_core::api::{Command, ErrorCode, Response, socket_name};

const MAX_CLIENTS: usize = 8;
const MAX_REQUEST_BYTES: usize = 16 * 1024 * 1024;
const IO_DEADLINE: Duration = Duration::from_secs(3);
const REPLY_DEADLINE: Duration = Duration::from_secs(15);
const POLL: Duration = Duration::from_millis(5);

pub struct Request {
    pub command: Command,
    pub reply: Sender<Response>,
    pub deadline: Instant,
}

fn would_block(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::WouldBlock
        // PIPE_NOWAIT reports ERROR_NO_DATA while a connected byte pipe is empty.
        || (cfg!(windows) && error.raw_os_error() == Some(232))
}

fn busy(message: &str) -> Response {
    Response::error(ErrorCode::Busy, message, None)
}

// Nonblocking named pipes must not enter interprocess's blocking flush-on-drop
// pool when a peer does not consume its response. Keep the pipe alive while the
// peer reads, then close it ourselves at the bounded deadline.
fn close(connection: Stream) {
    #[cfg(windows)]
    {
        let Stream::NamedPipe(pipe) = &connection;
        pipe.inner().assume_flushed();
    }
    drop(connection);
}

fn read_request(connection: &mut impl Read, deadline: Instant) -> Result<Vec<u8>, Box<Response>> {
    let mut line = Vec::new();
    let mut buffer = [0; 8192];
    loop {
        if Instant::now() >= deadline {
            return Err(Box::new(busy("API request read timed out")));
        }
        match connection.read(&mut buffer) {
            Ok(0) if cfg!(windows) => std::thread::sleep(POLL),
            Ok(0) => {
                return Err(Box::new(Response::error(
                    ErrorCode::InvalidCommand,
                    "Incomplete API request",
                    None,
                )));
            }
            Ok(count) => {
                let count = buffer[..count]
                    .iter()
                    .position(|byte| *byte == b'\n')
                    .map_or(count, |index| index + 1);
                if line.len() + count > MAX_REQUEST_BYTES {
                    return Err(Box::new(Response::error(
                        ErrorCode::InvalidCommand,
                        "API request exceeds 16777216 bytes",
                        None,
                    )));
                }
                line.extend_from_slice(&buffer[..count]);
                if line.last() == Some(&b'\n') {
                    return Ok(line);
                }
            }
            Err(error) if would_block(&error) => std::thread::sleep(POLL),
            Err(error) => {
                return Err(Box::new(Response::error(
                    ErrorCode::InvalidCommand,
                    format!("Could not read API request: {error}"),
                    None,
                )));
            }
        }
    }
}

#[cfg(windows)]
fn peer_closed(connection: &Stream) -> bool {
    use std::os::windows::io::{AsHandle, AsRawHandle};
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn PeekNamedPipe(
            handle: *mut std::ffi::c_void,
            buffer: *mut std::ffi::c_void,
            size: u32,
            read: *mut u32,
            available: *mut u32,
            remaining: *mut u32,
        ) -> i32;
    }
    let Stream::NamedPipe(pipe) = connection;
    // No buffer and no transfer: this only queries whether the peer still owns
    // its end. A nonblocking empty read alone also returns zero while connected.
    unsafe {
        PeekNamedPipe(
            pipe.as_handle().as_raw_handle(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        ) == 0
    }
}

fn reply(connection: &mut Stream, response: Response) {
    let Ok(mut json) = serde_json::to_vec(&response) else {
        return;
    };
    json.push(b'\n');
    let deadline = Instant::now() + IO_DEADLINE;
    let mut sent = 0;
    while sent < json.len() && Instant::now() < deadline {
        match connection.write(&json[sent..]) {
            Ok(0) => std::thread::sleep(POLL),
            Ok(count) => sent += count,
            Err(error) if would_block(&error) => std::thread::sleep(POLL),
            Err(_) => return,
        }
    }
    // Preserve unread buffered bytes for cooperating clients without a blocking
    // FlushFileBuffers; abandon unresponsive clients at the same I/O deadline.
    let mut byte = [0];
    while Instant::now() < deadline {
        #[cfg(windows)]
        if peer_closed(connection) {
            return;
        }
        match connection.read(&mut byte) {
            Ok(0) if cfg!(windows) => {}
            Ok(0) => return,
            Err(error) if would_block(&error) => {}
            Err(_) => return,
            Ok(_) => {}
        }
        std::thread::sleep(POLL);
    }
}

fn serve(connection: &mut Stream, sender: &SyncSender<Request>) -> Response {
    let line = match read_request(connection, Instant::now() + IO_DEADLINE) {
        Ok(line) => line,
        Err(response) => return *response,
    };
    match serde_json::from_slice::<Command>(&line) {
        Ok(command) => {
            let (reply, result) = mpsc::channel();
            let deadline = Instant::now() + REPLY_DEADLINE;
            match sender.try_send(Request {
                command,
                reply,
                deadline,
            }) {
                Ok(()) => result
                    .recv_timeout(REPLY_DEADLINE)
                    .unwrap_or_else(|_| busy("API reply timed out")),
                Err(mpsc::TrySendError::Full(_)) => busy("API command queue is full"),
                Err(mpsc::TrySendError::Disconnected(_)) => busy("API command queue is closed"),
            }
        }
        Err(error) => Response::error(
            ErrorCode::InvalidCommand,
            format!("Invalid command: {error}"),
            None,
        ),
    }
}

pub fn start() -> io::Result<Receiver<Request>> {
    start_named(&socket_name())
}

fn start_named(socket: &str) -> io::Result<Receiver<Request>> {
    let name = socket.to_ns_name::<GenericNamespaced>()?;
    let listener = ListenerOptions::new().name(name).create_sync()?;
    let (sender, receiver) = mpsc::sync_channel(MAX_CLIENTS);
    let active = Arc::new(AtomicUsize::new(0));
    std::thread::spawn(move || {
        for connection in listener.incoming() {
            let Ok(mut connection) = connection else {
                std::thread::sleep(POLL);
                continue;
            };
            if connection.set_nonblocking(true).is_err() {
                close(connection);
                continue;
            }
            if active
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                    (count < MAX_CLIENTS).then_some(count + 1)
                })
                .is_err()
            {
                // Tiny nonblocking overload response: never spawn another worker.
                let response = busy("API client limit reached");
                if let Ok(mut json) = serde_json::to_vec(&response) {
                    json.push(b'\n');
                    let _ = connection.write(&json);
                }
                close(connection);
                continue;
            }
            let sender = sender.clone();
            let active = active.clone();
            std::thread::spawn(move || {
                let response = serve(&mut connection, &sender);
                reply(&mut connection, response);
                close(connection);
                active.fetch_sub(1, Ordering::AcqRel);
            });
        }
    });
    Ok(receiver)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_admission_and_incomplete_deadline_release_clients() {
        let socket = format!(
            "afio-ipc-regression-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let receiver = start_named(&socket).unwrap();
        let mut clients = Vec::new();
        for _ in 0..12 {
            let name = socket.as_str().to_ns_name::<GenericNamespaced>().unwrap();
            let mut stream = Stream::connect(name).unwrap();
            stream.set_nonblocking(true).unwrap();
            let _ = stream.write(b"{");
            clients.push(stream);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(receiver.try_recv().is_err());
        for stream in &mut clients[8..] {
            let line = read_request(stream, Instant::now() + IO_DEADLINE).unwrap();
            let response: Response = serde_json::from_slice(&line).unwrap();
            assert_eq!(response.error.unwrap().message, "API client limit reached");
        }
        for stream in &mut clients[..8] {
            let line = read_request(stream, Instant::now() + IO_DEADLINE + IO_DEADLINE).unwrap();
            let response: Response = serde_json::from_slice(&line).unwrap();
            assert_eq!(
                response.error.unwrap().message,
                "API request read timed out"
            );
        }
        for stream in clients {
            close(stream);
        }
        std::thread::sleep(Duration::from_millis(50));
        let name = socket.as_str().to_ns_name::<GenericNamespaced>().unwrap();
        let mut stream = Stream::connect(name).unwrap();
        stream.set_nonblocking(true).unwrap();
        stream.write_all(b"{\"command\":\"status\"}\n").unwrap();
        let request = receiver.recv_timeout(IO_DEADLINE).unwrap();
        request.reply.send(Response::ok(None)).unwrap();
        let line = read_request(&mut stream, Instant::now() + IO_DEADLINE).unwrap();
        let response: Response = serde_json::from_slice(&line).unwrap();
        assert!(response.error.is_none());
        close(stream);
    }
    #[test]
    fn accepts_exactly_one_bounded_line() {
        let mut input = io::Cursor::new(b"{}\nignored");
        assert_eq!(
            read_request(&mut input, Instant::now() + IO_DEADLINE).unwrap(),
            b"{}\n"
        );
    }
    #[test]
    fn rejects_oversize_without_unbounded_input() {
        let mut input = io::repeat(b'x');
        let error = read_request(&mut input, Instant::now() + IO_DEADLINE).unwrap_err();
        assert_eq!(
            error.error.unwrap().message,
            "API request exceeds 16777216 bytes"
        );
    }
    #[test]
    fn expired_read_does_not_touch_input() {
        struct Unread;
        impl Read for Unread {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                panic!("expired read");
            }
        }
        let error = read_request(&mut Unread, Instant::now()).unwrap_err();
        assert_eq!(error.error.unwrap().message, "API request read timed out");
    }
}
