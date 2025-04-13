use std::path::PathBuf;
use std::sync::LazyLock;

pub static DEFAULT_SOCKET: LazyLock<PathBuf> = LazyLock::new(|| {
    runtime_dir().join("bus")
});

pub fn runtime_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("RUNTIME_DIRECTORY") {
        PathBuf::from(dir)
    } else {
        PathBuf::from("/var/run/samsunghvac")
    }
}
