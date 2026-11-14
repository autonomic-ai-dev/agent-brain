use std::path::PathBuf;

pub fn autonomic_root() -> PathBuf {
    std::env::var("AUTONOMIC_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home_dir().join(".autonomic"))
}

fn home_dir() -> PathBuf {
    dirs::home_dir()
        .or_else(|| std::env::var("HOME").ok().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn memory_dir() -> PathBuf {
    autonomic_root().join("memory")
}
