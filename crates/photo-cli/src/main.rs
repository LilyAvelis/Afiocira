use interprocess::local_socket::{GenericNamespaced, Stream, prelude::*};
use photo_core::api::{Command, Pane, Response, socket_name};
use photo_core::{Point, Size};
use std::env;
use std::ffi::OsString;
use std::io::{self, BufRead, Write};
use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let arguments: Vec<OsString> = env::args_os().skip(1).collect();
    if arguments.first().is_some_and(|arg| arg == "ctl") {
        return control(&arguments[1..]);
    }
    if arguments.first().is_some_and(|arg| arg == "ctl-json") {
        if arguments.len() != 1 {
            print_usage();
            return ExitCode::FAILURE;
        }
        return control_json();
    }
    let mut args = arguments.into_iter();
    let (Some(command), Some(folder), None) = (args.next(), args.next(), args.next()) else {
        print_usage();
        return ExitCode::FAILURE;
    };
    match command.to_str() {
        Some("list") => match photo_core::list_photos(Path::new(&folder)) {
            Ok(photos) => {
                for photo in &photos {
                    println!("{}", photo.path.display());
                }
                eprintln!("{} photos", photos.len());
                ExitCode::SUCCESS
            }
            Err(error) => failure(error),
        },
        Some("browse") => match photo_core::Gallery::open(Path::new(&folder)) {
            Ok(gallery) => browse(gallery),
            Err(error) => failure(error),
        },
        Some("open") => match photo_core::Gallery::open_file(Path::new(&folder)) {
            Ok(gallery) => browse(gallery),
            Err(error) => failure(error),
        },
        _ => {
            print_usage();
            ExitCode::FAILURE
        }
    }
}

fn control(args: &[OsString]) -> ExitCode {
    let command = match args {
        [action] if action == "status" => Command::Status,
        [action] if action == "enter-album" => Command::EnterAlbum,
        [action] if action == "navigator-snapshot" => Command::NavigatorSnapshot,
        [action] if action == "nested" => Command::LoadNested { deeper: false },
        [action] if action == "deeper" => Command::LoadNested { deeper: true },
        [action] if action == "cancel-image" => Command::CancelImage,
        [action] if action == "search" => Command::ShowSearch,
        [action] if action == "close-search" => Command::CloseSearch,
        [action, query] if action == "search-files" => Command::SearchFiles {
            query: query.to_string_lossy().into_owned(),
        },
        [action, index] if action == "open-result" => {
            let Some(index) = index.to_str().and_then(|s| s.parse().ok()) else {
                return failure(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Expected result index",
                ));
            };
            Command::OpenSearchResult { index }
        }
        [action] if action == "commands" => Command::ListCommands,
        [action] if action == "paste" => Command::RunAction {
            id: photo_core::actions::ActionId::Paste,
        },
        [action] if action == "delete" => Command::RunAction {
            id: photo_core::actions::ActionId::Delete,
        },
        [action] if action == "confirm-delete" => Command::ConfirmDelete,
        [action] if action == "cancel-delete" => Command::CancelDelete,
        [action] if action == "begin-rename" => Command::BeginRename,
        [action] if action == "cancel-rename" => Command::CancelRename,
        [action, name] if action == "rename" => Command::Rename {
            new_name: name.to_string_lossy().into_owned(),
        },
        [action, paths @ ..] if action == "import" => Command::ImportFiles {
            paths: paths.iter().map(Into::into).collect(),
        },
        [action, paths @ ..] if action == "drop-open" => Command::OpenDropped {
            paths: paths.iter().map(Into::into).collect(),
        },
        [action] if action == "actions" => Command::ShowCommands,
        [action] if action == "close-actions" => Command::CloseCommands,
        [action, query] if action == "search-actions" => Command::SearchCommands {
            query: query.to_string_lossy().into_owned(),
        },
        [action] if action == "duplicate" => Command::RunAction {
            id: photo_core::actions::ActionId::Duplicate,
        },
        [action] if action == "copy" => Command::RunAction {
            id: photo_core::actions::ActionId::Copy,
        },
        [action, id] if action == "action" => {
            let id = serde_json::from_value(serde_json::Value::String(
                id.to_string_lossy().into_owned(),
            ));
            let Ok(id) = id else {
                return failure(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Unknown action id",
                ));
            };
            Command::RunAction { id }
        }
        [action] if action == "video-play" => Command::VideoPause { paused: false },
        [action] if action == "video-pause" => Command::VideoPause { paused: true },
        [action, value] if action == "video-seek" || action == "video-volume" => {
            let Some(value) = value
                .to_str()
                .and_then(|s| s.parse::<f64>().ok())
                .filter(|v| v.is_finite())
            else {
                return failure(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Expected a finite number",
                ));
            };
            if action == "video-seek" {
                Command::VideoSeek { seconds: value }
            } else {
                Command::VideoVolume { volume: value }
            }
        }
        [action] if action == "list-order" => Command::ListOrder,
        [action, path] if action == "open" => Command::Open { path: path.into() },
        [action, index] if action == "select" => {
            let Some(index) = index.to_str().and_then(|value| value.parse().ok()) else {
                print_usage();
                return ExitCode::FAILURE;
            };
            Command::Select { index }
        }
        [action, path] if action == "select-path" => Command::SelectPath { path: path.into() },
        [action, index] if action == "toggle-selection" => {
            let Some(index) = index.to_str().and_then(|value| value.parse().ok()) else {
                print_usage();
                return ExitCode::FAILURE;
            };
            Command::ToggleSelection { index }
        }
        [action, index] if action == "extend-selection" => {
            let Some(index) = index.to_str().and_then(|value| value.parse().ok()) else {
                print_usage();
                return ExitCode::FAILURE;
            };
            Command::ExtendSelection { index }
        }
        [action, paths @ ..] if action == "mark" => Command::SetMarked {
            paths: paths.iter().map(Into::into).collect(),
        },
        [action, paths @ ..] if action == "order" => Command::SetOrder {
            paths: paths.iter().map(Into::into).collect(),
        },
        [action] if action == "open-selected" => Command::OpenSelected,
        [action] if action == "next" => Command::Next,
        [action] if action == "previous" => Command::Previous,
        [action] if action == "back" => Command::Back,
        [action] if action == "refresh" => Command::Refresh,
        [action] if action == "show-navigator" => Command::SetNavigatorVisible { visible: true },
        [action] if action == "hide-navigator" => Command::SetNavigatorVisible { visible: false },
        [action] if action == "focus-gallery" => Command::FocusPane {
            pane: Pane::Gallery,
        },
        [action] if action == "focus-navigator" => Command::FocusPane {
            pane: Pane::Navigator,
        },
        [action] if action == "reset-zoom" => Command::ResetZoom,
        [
            action,
            factor,
            x,
            y,
            width,
            height,
            fitted_width,
            fitted_height,
        ] if action == "zoom" => {
            let values = [factor, x, y, width, height, fitted_width, fitted_height];
            let Some([factor, x, y, width, height, fitted_width, fitted_height]) =
                parse_os_values(values)
            else {
                print_usage();
                return ExitCode::FAILURE;
            };
            Command::ZoomAt {
                multiplier: factor,
                anchor: Point { x, y },
                viewport: Size { width, height },
                fitted: Size {
                    width: fitted_width,
                    height: fitted_height,
                },
            }
        }
        [action, x, y, width, height, fitted_width, fitted_height] if action == "pan" => {
            let values = [x, y, width, height, fitted_width, fitted_height];
            let Some([x, y, width, height, fitted_width, fitted_height]) = parse_os_values(values)
            else {
                print_usage();
                return ExitCode::FAILURE;
            };
            Command::PanBy {
                delta: Point { x, y },
                viewport: Size { width, height },
                fitted: Size {
                    width: fitted_width,
                    height: fitted_height,
                },
            }
        }
        _ => {
            print_usage();
            return ExitCode::FAILURE;
        }
    };
    run_live_command(&command)
}

fn control_json() -> ExitCode {
    let mut line = String::new();
    if let Err(error) = io::stdin().read_line(&mut line) {
        eprintln!("Could not read JSON command: {error}");
        return ExitCode::FAILURE;
    }
    let command: Command = match serde_json::from_str(line.trim_start_matches('\u{feff}')) {
        Ok(command) => command,
        Err(error) => {
            eprintln!("Invalid JSON command: {error}");
            return ExitCode::FAILURE;
        }
    };
    run_live_command(&command)
}

fn run_live_command(command: &Command) -> ExitCode {
    match send_to_gui(command) {
        Ok(response) => {
            println!("{}", serde_json::to_string_pretty(&response).unwrap());
            if response.error.is_some() {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(error) => {
            eprintln!("Could not reach the running gallery: {error}");
            ExitCode::FAILURE
        }
    }
}

fn send_to_gui(command: &Command) -> io::Result<Response> {
    let socket = socket_name();
    let name = socket.to_ns_name::<GenericNamespaced>()?;
    let mut stream = Stream::connect(name)?;
    serde_json::to_writer(&mut stream, command)?;
    stream.write_all(b"\n")?;
    stream.flush()?;
    let mut line = String::new();
    io::BufReader::new(stream).read_line(&mut line)?;
    serde_json::from_str(&line).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn browse(mut gallery: photo_core::Gallery) -> ExitCode {
    println!(
        "{} photos in {}",
        gallery.photos().len(),
        gallery.folder().display()
    );
    println!(
        "Commands: open <index>, next, previous, back, anchor <index>, zoom <factor> <x> <y> <viewport-w> <viewport-h> <fitted-w> <fitted-h>, pan <dx> <dy> <viewport-w> <viewport-h> <fitted-w> <fitted-h>, reset-zoom, status, quit"
    );
    print_state(&gallery);
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();

    for line in stdin.lock().lines() {
        let Ok(line) = line else {
            eprintln!("Could not read command");
            return ExitCode::FAILURE;
        };
        let parts: Vec<_> = line
            .trim_start_matches('\u{feff}')
            .split_whitespace()
            .collect();
        if parts.first() == Some(&"zoom") {
            match parse_values::<7>(&parts[1..]) {
                Some([factor, x, y, width, height, fitted_width, fitted_height])
                    if gallery.zoom_at(
                        factor,
                        photo_core::Point { x, y },
                        photo_core::Size { width, height },
                        photo_core::Size {
                            width: fitted_width,
                            height: fitted_height,
                        },
                    ) =>
                {
                    print_state(&gallery)
                }
                _ => println!("Invalid zoom arguments or not viewing a photo"),
            }
            continue;
        }
        if parts.first() == Some(&"pan") {
            match parse_values::<6>(&parts[1..]) {
                Some([x, y, width, height, fitted_width, fitted_height])
                    if gallery.pan_by(
                        photo_core::Point { x, y },
                        photo_core::Size { width, height },
                        photo_core::Size {
                            width: fitted_width,
                            height: fitted_height,
                        },
                    ) =>
                {
                    print_state(&gallery)
                }
                _ => println!("Invalid pan arguments or not viewing a photo"),
            }
            continue;
        }
        if parts.len() > 2 {
            println!("Too many arguments");
            continue;
        }
        let command = parts.first().copied();
        let argument = parts.get(1).copied();
        match (command, argument) {
            (Some("open"), Some(index)) => match index.parse::<usize>() {
                Ok(index) if gallery.open_photo(index) => print_state(&gallery),
                _ => println!("Invalid photo index"),
            },
            (Some("anchor"), Some(index)) => match index.parse::<usize>() {
                Ok(index) if gallery.set_grid_anchor(index) => print_state(&gallery),
                _ => println!("Invalid grid index"),
            },
            (Some("next"), None) => {
                if gallery.next_photo() {
                    print_state(&gallery);
                } else {
                    println!("No next photo");
                }
            }
            (Some("previous"), None) => {
                if gallery.previous_photo() {
                    print_state(&gallery);
                } else {
                    println!("No previous photo");
                }
            }
            (Some("back"), None) => {
                gallery.back_to_grid();
                print_state(&gallery);
            }
            (Some("status"), None) => print_state(&gallery),
            (Some("reset-zoom"), None) => {
                gallery.reset_zoom();
                print_state(&gallery);
            }
            (Some("quit"), None) => return ExitCode::SUCCESS,
            _ => println!("Unknown command"),
        }
        if stdout.flush().is_err() {
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}

fn print_state(gallery: &photo_core::Gallery) {
    let selected = gallery
        .selected_photo()
        .map(|photo| photo.name.as_str())
        .unwrap_or("none");
    println!(
        "view={:?} selected={:?} name={} grid_anchor={} zoom={:.3} offset=({:.1},{:.1})",
        gallery.view(),
        gallery.selected_index(),
        selected,
        gallery.grid_anchor(),
        gallery.zoom().factor,
        gallery.zoom().offset.x,
        gallery.zoom().offset.y
    );
}

fn parse_values<const N: usize>(values: &[&str]) -> Option<[f32; N]> {
    if values.len() != N {
        return None;
    }
    let mut result = [0.0; N];
    for (index, value) in values.iter().enumerate() {
        result[index] = value.parse().ok()?;
    }
    Some(result)
}

fn parse_os_values<const N: usize>(values: [&OsString; N]) -> Option<[f32; N]> {
    let mut result = [0.0; N];
    for (index, value) in values.into_iter().enumerate() {
        result[index] = value.to_str()?.parse().ok()?;
    }
    Some(result)
}

fn print_usage() {
    eprintln!(
        "Usage: photo-cli list <folder> | browse <folder> | open <file> | ctl <status|list-order|open PATH|select INDEX|select-path PATH|extend-selection INDEX|toggle-selection INDEX|mark [PATH ...]|order [PATH ...]|open-selected|next|previous|back|refresh|show-navigator|hide-navigator|focus-gallery|focus-navigator|zoom FACTOR X Y VIEW_W VIEW_H FIT_W FIT_H|pan DX DY VIEW_W VIEW_H FIT_W FIT_H|reset-zoom|commands|actions|close-actions|search-actions QUERY|action ID|copy|duplicate|paste|delete|confirm-delete|cancel-delete|begin-rename|rename NEW_NAME|cancel-rename|import PATH...|drop-open PATH...|video-play|video-pause|video-seek SECONDS|video-volume 0..100> | ctl-json < JSON_COMMAND"
    );
}

fn failure(error: io::Error) -> ExitCode {
    eprintln!("Could not open path: {error}");
    ExitCode::FAILURE
}
