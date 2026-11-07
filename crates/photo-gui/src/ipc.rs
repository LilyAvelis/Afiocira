use std::io::{self, BufRead, BufReader, Write};
use std::sync::mpsc::{self, Receiver, Sender};

use interprocess::local_socket::{GenericNamespaced, ListenerOptions, prelude::*};
use photo_core::api::{Command, ErrorCode, Response, socket_name};

pub struct Request {
    pub command: Command,
    pub reply: Sender<Response>,
}

pub fn start() -> io::Result<Receiver<Request>> {
    let socket = socket_name();
    let name = socket.to_ns_name::<GenericNamespaced>()?;
    let listener = ListenerOptions::new().name(name).create_sync()?;
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        for connection in listener.incoming() {
            let Ok(mut connection) = connection else {
                break;
            };
            let sender = sender.clone();
            std::thread::spawn(move || {
                let mut line = String::new();
                if BufReader::new(&mut connection)
                    .read_line(&mut line)
                    .is_err()
                {
                    return;
                }
                let response = match serde_json::from_str::<Command>(&line) {
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
