//! Explicit adapter-owned process creation inputs. No opaque `Command` conversion.
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

/// Standard-stream policy for an owned child.
#[derive(Clone, Copy, Debug, Default)]
pub enum OwnedStdio {
    Null,
    Piped,
    #[default]
    Inherit,
}

/// Structured process inputs consumed by the platform ownership adapter.
/// Arguments and environment values are never included in adapter diagnostics.
pub struct OwnedSpawnSpec {
    pub(crate) executable: OsString,
    pub(crate) arguments: Vec<OsString>,
    pub(crate) directory: Option<PathBuf>,
    pub(crate) clear_environment: bool,
    pub(crate) environment: Vec<(OsString, Option<OsString>)>,
    pub(crate) streams: [OwnedStdio; 3],
}

impl OwnedSpawnSpec {
    pub fn new(executable: impl AsRef<OsStr>) -> Self {
        Self {
            executable: executable.as_ref().into(),
            arguments: Vec::new(),
            directory: None,
            clear_environment: false,
            environment: Vec::new(),
            streams: [OwnedStdio::Inherit; 3],
        }
    }
    pub fn arg(&mut self, argument: impl AsRef<OsStr>) -> &mut Self {
        self.arguments.push(argument.as_ref().into());
        self
    }
    pub fn args<I, S>(&mut self, arguments: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.arguments
            .extend(arguments.into_iter().map(|s| s.as_ref().into()));
        self
    }
    pub fn current_dir(&mut self, directory: impl AsRef<Path>) -> &mut Self {
        self.directory = Some(directory.as_ref().into());
        self
    }
    pub fn env(&mut self, key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> &mut Self {
        self.environment
            .push((key.as_ref().into(), Some(value.as_ref().into())));
        self
    }
    pub fn env_remove(&mut self, key: impl AsRef<OsStr>) -> &mut Self {
        self.environment.push((key.as_ref().into(), None));
        self
    }
    pub fn env_clear(&mut self) -> &mut Self {
        self.clear_environment = true;
        self
    }
    pub fn stdin(&mut self, stream: OwnedStdio) -> &mut Self {
        self.streams[0] = stream;
        self
    }
    pub fn stdout(&mut self, stream: OwnedStdio) -> &mut Self {
        self.streams[1] = stream;
        self
    }
    pub fn stderr(&mut self, stream: OwnedStdio) -> &mut Self {
        self.streams[2] = stream;
        self
    }
}
