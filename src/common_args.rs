use std::{
    path::{Component, Path, PathBuf},
    time::Duration,
};

use clap::{Args, ValueHint};

#[derive(Debug, Args)]
pub(crate) struct Arguments {
    /// Dir where images should be temporarily stored. Should be tmpfs.
    #[arg(long, value_hint=ValueHint::DirPath)]
    pub work_dir: PathBuf,

    /// JPEG file to look for in directory
    #[arg(long, default_value = "current.jpg")]
    pub filename: PathBuf,

    #[arg(long, default_value = None)]
    pub max_fps: Option<f32>,
}

impl Arguments {
    pub fn max_fps_delay(&self) -> Option<Duration> {
        self.max_fps.map(|fps| Duration::from_secs_f32(1f32 / fps))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ArgumentsError {
    #[error("Invalid filename: {0}")]
    InvalidFilename(PathBuf),
}

pub(crate) fn assert_filename_only(path: &Path) -> Result<&Path, ArgumentsError> {
    let mut components = path.components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(filename)), None) => Ok(Path::new(filename)),
        _ => Err(ArgumentsError::InvalidFilename(path.to_path_buf())),
    }
}
