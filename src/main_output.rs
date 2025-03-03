use std::time::Duration;

use clap::{Parser, builder::TypedValueParser};
use common_args::assert_filename_only;
use inotify::{Inotify, WatchMask};
use tokio::{fs::OpenOptions, io::AsyncWriteExt, select, time::Instant};
use tokio_stream::StreamExt as _;

mod common_args;
mod event_source;

use event_source::EventSource;

#[derive(Debug, Clone)]
enum ListenType {
    #[allow(dead_code)]
    Port(u16),
    Stdio,
}

#[derive(Clone, Copy)]
struct ListenTypeParser;

impl TypedValueParser for ListenTypeParser {
    type Value = ListenType;

    fn parse_ref(
        &self,
        cmd: &clap::Command,
        arg: Option<&clap::Arg>,
        value: &std::ffi::OsStr,
    ) -> Result<Self::Value, clap::Error> {
        if value == "stdio" {
            Ok(ListenType::Stdio)
        } else {
            let inner = clap::value_parser!(u16);
            let value = inner.parse_ref(cmd, arg, value)?;
            Ok(ListenType::Port(value))
        }
    }

    fn possible_values(
        &self,
    ) -> Option<Box<dyn Iterator<Item = clap::builder::PossibleValue> + '_>> {
        Some(Box::new(
            ["stdio", "port_number"]
                .into_iter()
                .map(clap::builder::PossibleValue::new),
        ))
    }
}

#[derive(Debug, Parser)]
struct MjpgMultiplexOutput {
    #[command(flatten)]
    shared: common_args::Arguments,

    #[arg(hide = true, env = "SD_LISTEN_FDS_START")]
    systemd_listen_fds_start: Option<usize>,

    #[arg(long, required_unless_present("systemd_listen_fds_start"), value_parser = ListenTypeParser)]
    listen_port: Option<ListenType>,

    #[arg(long, required = true)]
    boundary: std::ffi::OsString,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), anyhow::Error> {
    let mut buffer = [0; 1024];

    let mut inotify = EventSource::new(Inotify::init()?, &mut buffer);
    let args = MjpgMultiplexOutput::parse();

    // Watch for modify and close events.
    let file_moved = inotify
        .as_inotify()
        .watches()
        .add(&args.shared.work_dir, WatchMask::MOVED_TO)?;
    let input_file_path = args
        .shared
        .work_dir
        .join(assert_filename_only(&args.shared.filename)?);
    let boundary = {
        let mut b: Vec<u8> = vec![b'-', b'-'];
        b.append(&mut args.boundary.as_encoded_bytes().into());
        b.append(&mut b"\nContent-Type: image/jpg\n\n".into());
        b
    };
    args.boundary.as_encoded_bytes();

    let mut stdout = tokio::io::stdout();

    let event_stream = inotify.as_event_stream()?;
    let sleep_duration = args.shared.max_fps_delay();
    let async_first_sleep = async {
        tokio::time::sleep_until(Instant::now() + sleep_duration.unwrap_or(Duration::from_secs(1)))
            .await;
    };
    tokio::pin!(async_first_sleep);
    let mut first_sleep_done = false;
    'next_frame: loop {
        let sleep_until = {
            let until = sleep_duration.map(|duration| Instant::now() + duration);
            async move {
                if let Some(until) = until {
                    tokio::time::sleep_until(until).await;
                }
            }
        };
        tokio::pin!(sleep_until);
        let open_file = async {
            let mut file = OpenOptions::new()
                .create(false)
                .read(true)
                .write(false)
                .open(&input_file_path)
                .await?;
            stdout.write_all(&boundary).await?;
            tokio::io::copy(&mut file, &mut stdout).await?;
            stdout.write_all(b"\n").await?;
            stdout.flush().await?;
            Ok::<_, std::io::Error>(file)
        };
        tokio::pin!(open_file);
        let mut can_advance = false;
        let mut new_file = false;
        let mut keep_file_open = None;
        loop {
            select! {
                _ = &mut async_first_sleep, if !first_sleep_done => {
                    first_sleep_done = true;
                    new_file = true;
                },
                file = &mut open_file, if new_file && keep_file_open.is_none() => {
                    let file = match file {
                        Ok(file) => file,
                        Err(err) => {
                            eprintln!("Read file failed: {err}");
                            continue 'next_frame;
                        }
                    };
                    keep_file_open = Some(file);
                }
                _ = &mut sleep_until, if !can_advance && sleep_duration.is_some() => {
                    can_advance = true;
                },
                event = event_stream.try_next(), if !new_file => {
                    let Some(event) = event? else {
                        eprintln!("Event stream dried up");
                        return Ok(());
                    };
                    if event.wd == file_moved {
                        match event.name {
                            Some(filename) if Some(filename.as_ref()) == input_file_path.file_name() => {
                                first_sleep_done = true;
                                new_file = true;
                            }
                            Some(filename) => {
                                eprintln!("Invalid filename {}", filename.to_string_lossy());
                            }
                            None => {
                                eprintln!("inotify event without filename");
                            }
                        }
                    }
                }
                else => {
                    debug_assert!(can_advance);
                    drop(keep_file_open);
                    continue 'next_frame;
                }
            }
        }
    }
}
