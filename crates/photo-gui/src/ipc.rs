use std::io::{self, BufRead, BufReader, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};

use interprocess::local_socket::{GenericNamespaced, ListenerOptions, prelude::*};
use photo_core::api::{Command, ErrorCode, Response, SOCKET_NAME};

pub struct Request {
    pub command: Command,
    pub reply: Sender<Response>,
}

const MAX_COMMAND_BYTES: usize = 16 * 1024 * 1024;
const MAX_CLIENTS: usize = 8;

struct ClientPermit(Arc<AtomicUsize>);

impl Drop for ClientPermit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

fn read_command_line<R: BufRead>(reader: &mut R, limit: usize) -> io::Result<Vec<u8>> {
    let mut line = Vec::new();
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "command has no newline",
            ));
        }
        let length = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(available.len(), |index| index + 1);
        if length > limit.saturating_sub(line.len()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "command exceeds 16 MiB",
            ));
        }
        let done = available[length - 1] == b'\n';
        line.extend_from_slice(&available[..length]);
        reader.consume(length);
        if done {
            return Ok(line);
        }
    }
}

pub fn start() -> io::Result<Receiver<Request>> {
    let name = SOCKET_NAME.to_ns_name::<GenericNamespaced>()?;
    let listener = ListenerOptions::new().name(name).create_sync()?;
    let (sender, receiver) = mpsc::channel();
    let active = Arc::new(AtomicUsize::new(0));
    std::thread::spawn(move || {
        for connection in listener.incoming() {
            let Ok(mut connection) = connection else {
                break;
            };
            if active.fetch_add(1, Ordering::Relaxed) >= MAX_CLIENTS {
                active.fetch_sub(1, Ordering::Relaxed);
                continue;
            }
            let permit = ClientPermit(Arc::clone(&active));
            let sender = sender.clone();
            std::thread::spawn(move || {
                let _permit = permit;
                let line = match read_command_line(
                    &mut BufReader::new(&mut connection),
                    MAX_COMMAND_BYTES,
                ) {
                    Ok(line) => line,
                    Err(error) if error.kind() == io::ErrorKind::InvalidData => {
                        let response =
                            Response::error(ErrorCode::InvalidCommand, error.to_string(), None);
                        if let Ok(json) = serde_json::to_string(&response) {
                            let _ = writeln!(connection, "{json}");
                        }
                        return;
                    }
                    Err(_) => return,
                };
                let response = match serde_json::from_slice::<Command>(&line) {
                    Ok(command) => {
                        let (reply, result) = mpsc::channel();
                        if sender.send(Request { command, reply }).is_err() {
                            return;
                        }
                        match result.recv() {
                            Ok(response) => response,
                            Err(_) => return,
                        }
                    }
                    Err(error) => Response::error(
                        ErrorCode::InvalidCommand,
                        format!("Invalid command: {error}"),
                        None,
                    ),
                };
                if let Ok(json) = serde_json::to_string(&response) {
                    let _ = writeln!(connection, "{json}");
                }
            });
        }
    });
    Ok(receiver)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_frame_has_a_size_limit_and_requires_newline() {
        assert_eq!(
            read_command_line(&mut io::Cursor::new(b"{}\n"), 3).unwrap(),
            b"{}\n"
        );
        assert_eq!(
            read_command_line(&mut io::Cursor::new(b"1234\n"), 4)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(
            read_command_line(&mut io::Cursor::new(b"{}"), 3)
                .unwrap_err()
                .kind(),
            io::ErrorKind::UnexpectedEof
        );
    }
}
