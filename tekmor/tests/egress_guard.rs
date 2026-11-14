use std::fs;
use std::path::{Path, PathBuf};

const NETWORK_CLIENT_MARKERS: [&str; 16] = [
    "ureq::",
    "reqwest::",
    "hyper::",
    "opendal::",
    "TcpStream",
    "TcpListener",
    "UdpSocket",
    "tokio::net::",
    "Command::new(\"curl\")",
    "Command::new(\"wget\")",
    "[\"fetch\"",
    "[\"clone\"",
    "[\"pull\"",
    "[\"push\"",
    "\"clone\",",
    "\"fetch\",",
];

const EXPECTED_NETWORK_FILES: [&str; 9] = [
    "tekmor/src/auto_update.rs",
    "tekmor/src/docs/fetch.rs",
    "tekmor/src/extensions.rs",
    "tekmor/src/mcp/server.rs",
    "tekmor/src/packages/mod.rs",
    "tekmor/src/registry_sync.rs",
    "tekmor/src/sync/cloud.rs",
    "tekmor/src/sync/git.rs",
    "extensions/bench/src/skills_sh.rs",
];

const LLM_PROVIDER_HOSTS: [&str; 4] = [
    "api.openai.com",
    "api.anthropic.com",
    "generativelanguage.googleapis.com",
    "api.mistral.ai",
];

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

fn source_files() -> Vec<PathBuf> {
    let mut files = Vec::new();
    rust_files(&crate_dir().join("src"), &mut files);
    rust_files(&crate_dir().join("../extensions"), &mut files);
    files
}

fn relative_to_repo(path: &Path) -> String {
    let repo = crate_dir().join("..");
    let canonical_repo = repo.canonicalize().expect("repo root");
    let canonical = path.canonicalize().expect("source file");
    canonical
        .strip_prefix(&canonical_repo)
        .expect("source file lives in the repo")
        .to_string_lossy()
        .replace('\\', "/")
}

#[test]
fn only_listed_files_open_network_connections() {
    let mut found: Vec<String> = source_files()
        .iter()
        .filter(|path| {
            let text = fs::read_to_string(path).unwrap_or_default();
            NETWORK_CLIENT_MARKERS
                .iter()
                .any(|marker| text.contains(marker))
        })
        .map(|path| relative_to_repo(path))
        .collect();
    found.sort();

    let mut expected: Vec<String> = EXPECTED_NETWORK_FILES
        .iter()
        .map(|s| s.to_string())
        .collect();
    expected.sort();

    assert_eq!(
        found, expected,
        "network client usage changed. Update docs/egress.md and EXPECTED_NETWORK_FILES together."
    );
}

#[test]
fn egress_statement_lists_every_network_file() {
    let statement = fs::read_to_string(crate_dir().join("docs/egress.md")).expect("docs/egress.md");
    for file in EXPECTED_NETWORK_FILES {
        let named = file.strip_prefix("tekmor/").unwrap_or(file);
        assert!(
            statement.contains(named),
            "docs/egress.md must name {named}, which opens network connections"
        );
    }
}

#[test]
fn no_telemetry_sdk_is_a_dependency() {
    const TELEMETRY_CRATES: [&str; 5] =
        ["sentry", "posthog", "opentelemetry", "segment", "mixpanel"];
    let manifests = [
        crate_dir().join("Cargo.toml"),
        crate_dir().join("../extensions/bench/Cargo.toml"),
    ];
    for manifest in manifests {
        let text = fs::read_to_string(&manifest).expect("read Cargo.toml");
        for name in TELEMETRY_CRATES {
            assert!(
                !text.contains(name),
                "{} depends on telemetry crate {name}; keep the no-telemetry statement true",
                manifest.display()
            );
        }
    }
}

#[test]
fn no_llm_provider_host_appears_in_source() {
    for path in source_files() {
        let text = fs::read_to_string(&path).unwrap_or_default();
        for host in LLM_PROVIDER_HOSTS {
            assert!(
                !text.contains(host),
                "{} references LLM provider host {host}; tekmor must not call model APIs",
                relative_to_repo(&path)
            );
        }
    }
}
