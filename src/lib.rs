pub mod config;
pub mod environment;
pub mod error;
pub mod executor;
pub mod git;
pub mod mcp;
#[cfg(feature = "github")]
pub mod pr_watch;
pub mod provider;
pub mod remote;
pub mod version;
pub mod watcher;
pub mod workspace;

pub mod prelude {
    pub use crate::config::Config;
    pub use crate::environment::{Environment, SkipperEnvironment};
    pub use crate::error::{CliError, Result};
    pub use crate::provider::{BuildRun, Provider, ProviderStatus, Registry};
    pub use crate::watcher::{
        GitStatusWatcher, Notification, Watcher, WatcherManager, WatcherState,
    };
}
