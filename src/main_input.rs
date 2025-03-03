use std::{path::PathBuf, time::Duration};

use clap::Parser;
use common_args::assert_filename_only;
use event_source::EventSource;
use inotify::{EventMask, Inotify, WatchDescriptor, WatchMask};
use tokio::{
    fs::{self, OpenOptions},
    io::AsyncWriteExt,
    select,
    time::Instant,
};
use tokio_stream::StreamExt as _;

mod common_args;
mod event_source;

#[derive(Debug, Parser)]
struct MjpgMultiplexInput {
    #[arg(long, default_value = "saving.jpg")]
    working_filename: PathBuf,

    #[command(flatten)]
    shared: common_args::Arguments,

    #[arg(long)]
    overwrite_existing_temp_file: bool,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), anyhow::Error> {
    let mut buffer = [0; 1024];
    let mut inotify = EventSource::new(
        Inotify::init().expect("Error while initializing inotify instance"),
        &mut buffer,
    );
    let args = MjpgMultiplexInput::parse();
    let output_file_path = args
        .shared
        .work_dir
        .join(assert_filename_only(&args.shared.filename)?);
    let tmp_file_path = args
        .shared
        .work_dir
        .join(assert_filename_only(&args.working_filename)?);

    let mut counter = 0;
    let sleep_duration = args
        .shared
        .max_fps
        .map(|fps| Duration::from_millis(1000u64 / u64::from(fps)));
    let mut file_watch: Option<WatchDescriptor> = None;
    let mut file_accessed = true;
    let mut file_closed = true;
    'next_frame: loop {
        let wait_for_next_frame = {
            let instant = sleep_duration.map(|duration| Instant::now() + duration);
            async move {
                if let Some(instant) = instant {
                    tokio::time::sleep_until(instant).await;
                }
            }
        };
        tokio::pin!(wait_for_next_frame);
        let current_file = async {
            let mut tmp_file = {
                let mut options = OpenOptions::new();
                options.write(true).create(true);
                if !args.overwrite_existing_temp_file {
                    options.create_new(true);
                }
                options.truncate(true).open(&tmp_file_path).await?
            };

            counter += 1;
            tmp_file
                .write_all(format!("File number {counter}").as_bytes())
                .await?;
            let result = tmp_file.flush().await.map(|_| tmp_file_path.clone());
            drop(tmp_file);
            result
        };
        tokio::pin!(current_file);
        let rename_file = async { fs::rename(&tmp_file_path, &output_file_path).await };
        tokio::pin!(rename_file);

        let mut next_file_watch = None;
        let mut file_created = false;
        let mut timer_ticked = false;
        loop {
            let events = inotify.as_event_stream()?;

            select! {
                new_file = &mut current_file, if !file_created && (file_accessed || file_closed) => {
                    file_created = true;
                    next_file_watch = inotify
                            .as_inotify()
                            .watches()
                            .add(new_file?, WatchMask::ACCESS | WatchMask::CLOSE_NOWRITE | WatchMask::OPEN)?
                            .into();
                },
                _ = &mut rename_file, if file_created && file_closed && timer_ticked => {
                    if let Some(file_watch) = file_watch.take() {
                        _ = inotify.as_inotify().watches().remove(file_watch);
                    }
                    file_watch = next_file_watch.take();
                    file_accessed = false;
                    file_closed = false;
                    continue 'next_frame;
                }
                _ = &mut wait_for_next_frame, if !timer_ticked => {
                    timer_ticked = true;
                }
                event = events.try_next(), if file_watch.is_some() => {
                    if let (Some(event), Some(file_watch)) = (event?, &file_watch) {
                        if event.wd == *file_watch {
                            if event.mask.intersects(EventMask::CLOSE_NOWRITE) {
                                file_closed = true;
                            }
                            if event.mask.intersects(EventMask::ACCESS | EventMask::OPEN) {
                                file_accessed = true;
                            }
                        }
                    }
                }
            }
        }
    }
}
