use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::registry_sync::cache_dir;

const PREFIX: &str = "agent-brain-";
const PARTIAL_SUFFIX: &str = ".partial";
const MAX_BINARY_BYTES: u64 = 256 * 1024 * 1024;
const TRUSTED_TEAM_ID: &str = "9HSMVVUMVX";

#[derive(Debug, Deserialize, Serialize)]
pub struct Manifest {
    pub name: String,
    pub version: String,
    pub description: String,
    pub subcommands: Vec<String>,
    pub binaries: HashMap<String, Artifact>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Artifact {
    pub url: String,
    pub sha256: String,
}

pub fn bin_dir(home: &Path) -> PathBuf {
    home.join("extensions").join("bin")
}

fn manifests_dir(home: &Path) -> PathBuf {
    home.join("extensions").join("manifests")
}

fn binary_path(home: &Path, name: &str) -> PathBuf {
    bin_dir(home).join(format!("{PREFIX}{name}"))
}

fn manifest_path(home: &Path, name: &str) -> PathBuf {
    manifests_dir(home).join(format!("{name}.json"))
}

fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        bail!("extension name `{name}` must contain only letters, digits, and dashes");
    }
    Ok(())
}

pub fn load_manifest(home: &Path, name: &str) -> Result<Manifest> {
    validate_name(name)?;
    let path = cache_dir(home)
        .join("repo")
        .join("extensions")
        .join(name)
        .join("extension.json");
    let raw = fs::read_to_string(&path).with_context(|| {
        format!("extension `{name}` is not in the agent-registry cache ({})", path.display())
    })?;
    let manifest: Manifest =
        serde_json::from_str(&raw).with_context(|| format!("parse {}", path.display()))?;
    if manifest.name != name {
        bail!("manifest name `{}` does not match `{name}`", manifest.name);
    }
    Ok(manifest)
}

fn read_installed_manifest(home: &Path, name: &str) -> Result<Manifest> {
    let path = manifest_path(home, name);
    let raw = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    serde_json::from_str(&raw).with_context(|| format!("parse {}", path.display()))
}

pub fn current_target() -> Result<&'static str> {
    match (std::env::consts::ARCH, std::env::consts::OS) {
        ("aarch64", "macos") => Ok("aarch64-apple-darwin"),
        ("x86_64", "macos") => Ok("x86_64-apple-darwin"),
        (arch, os) => bail!("no extension builds for {arch}-{os}; signature checks need macOS"),
    }
}

fn team_id_matches(codesign_details: &str, team_id: &str) -> bool {
    let wanted = format!("TeamIdentifier={team_id}");
    codesign_details.lines().any(|line| line.trim() == wanted)
}

fn verify_signature(path: &Path) -> Result<()> {
    let verified = Command::new("codesign")
        .args(["--verify", "--strict"])
        .arg(path)
        .status()
        .context("run codesign --verify")?;
    if !verified.success() {
        bail!("{} is not validly code-signed", path.display());
    }
    let details = Command::new("codesign")
        .args(["-dv", "--verbose=2"])
        .arg(path)
        .output()
        .context("run codesign -dv")?;
    let text = String::from_utf8_lossy(&details.stderr);
    if !team_id_matches(&text, TRUSTED_TEAM_ID) {
        bail!("{} is not signed by the trusted team {TRUSTED_TEAM_ID}", path.display());
    }
    Ok(())
}

pub fn verify_sha256(bytes: &[u8], expected: &str) -> Result<()> {
    let actual = format!("{:x}", Sha256::digest(bytes));
    if actual.eq_ignore_ascii_case(expected) {
        return Ok(());
    }
    bail!("sha256 mismatch: manifest says {expected}, download is {actual}")
}

fn download(url: &str) -> Result<Vec<u8>> {
    let response = ureq::get(url)
        .call()
        .with_context(|| format!("download {url}"))?;
    let mut bytes = Vec::new();
    response
        .into_body()
        .into_reader()
        .take(MAX_BINARY_BYTES + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("read {url}"))?;
    if bytes.len() as u64 > MAX_BINARY_BYTES {
        bail!("download {url} exceeds {MAX_BINARY_BYTES} bytes");
    }
    Ok(bytes)
}

pub fn install(home: &Path, name: &str) -> Result<PathBuf> {
    let manifest = load_manifest(home, name)?;
    let target = current_target()?;
    let artifact = manifest.binaries.get(target).with_context(|| {
        format!("extension `{name}` {} has no build for {target}", manifest.version)
    })?;
    let bytes = download(&artifact.url)?;
    verify_sha256(&bytes, &artifact.sha256)?;

    let dir = bin_dir(home);
    fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    let dest = binary_path(home, name);
    let partial = dir.join(format!("{PREFIX}{name}{PARTIAL_SUFFIX}"));
    fs::write(&partial, &bytes).with_context(|| format!("write {}", partial.display()))?;
    fs::set_permissions(&partial, fs::Permissions::from_mode(0o755))?;
    if let Err(err) = verify_signature(&partial) {
        let _ = fs::remove_file(&partial);
        return Err(err);
    }
    fs::rename(&partial, &dest).with_context(|| format!("install {}", dest.display()))?;

    let manifests = manifests_dir(home);
    fs::create_dir_all(&manifests).with_context(|| format!("create {}", manifests.display()))?;
    let json = serde_json::to_string_pretty(&manifest)?;
    fs::write(manifest_path(home, name), format!("{json}\n"))?;
    Ok(dest)
}

pub fn installed(home: &Path) -> Result<Vec<String>> {
    let dir = manifests_dir(home);
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut names: Vec<String> = fs::read_dir(&dir)?
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter_map(|file| file.strip_suffix(".json").map(str::to_string))
        .collect();
    names.sort();
    Ok(names)
}

pub fn owner_of(home: &Path, command: &str) -> Result<Option<String>> {
    for name in installed(home)? {
        if read_installed_manifest(home, &name)?
            .subcommands
            .iter()
            .any(|sub| sub == command)
        {
            return Ok(Some(name));
        }
    }
    Ok(None)
}

pub fn remove(home: &Path, name: &str) -> Result<()> {
    validate_name(name)?;
    let binary = binary_path(home, name);
    let manifest = manifest_path(home, name);
    if !binary.is_file() && !manifest.is_file() {
        bail!("extension `{name}` is not installed");
    }
    for path in [binary, manifest] {
        if path.is_file() {
            fs::remove_file(&path).with_context(|| format!("remove {}", path.display()))?;
        }
    }
    Ok(())
}

pub fn run(home: &Path, name: &str, args: &[String]) -> Result<i32> {
    let status = Command::new(binary_path(home, name))
        .args(args)
        .env("AGENT_BRAIN_HOME", home)
        .env("AGENT_BRAIN_SOCKET", home.join("serve.sock"))
        .status()
        .with_context(|| format!("run extension `{name}`"))?;
    Ok(status.code().unwrap_or(1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn write_manifest(home: &Path, name: &str, body: &str) {
        let dir = cache_dir(home).join("repo").join("extensions").join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("extension.json"), body).unwrap();
    }

    fn manifest_json(name: &str, subcommands: &[&str]) -> String {
        let subs = subcommands
            .iter()
            .map(|sub| format!("\"{sub}\""))
            .collect::<Vec<_>>()
            .join(",");
        format!(
            r#"{{"name":"{name}","version":"0.1.0","description":"d","subcommands":[{subs}],"binaries":{{"aarch64-apple-darwin":{{"url":"https://example.invalid/x","sha256":"00"}}}}}}"#
        )
    }

    fn record_installed(home: &Path, name: &str, subcommands: &[&str]) {
        let dir = manifests_dir(home);
        fs::create_dir_all(&dir).unwrap();
        fs::write(manifest_path(home, name), manifest_json(name, subcommands)).unwrap();
    }

    #[test]
    fn names_are_restricted_to_letters_digits_and_dashes() {
        assert!(validate_name("bench").is_ok());
        assert!(validate_name("skills-sh").is_ok());
        assert!(validate_name("../evil").is_err());
        assert!(validate_name("a/b").is_err());
        assert!(validate_name("").is_err());
    }

    #[test]
    fn sha256_accepts_the_published_digest_and_rejects_others() {
        let digest = format!("{:x}", Sha256::digest(b"payload"));
        assert!(verify_sha256(b"payload", &digest).is_ok());
        assert!(verify_sha256(b"payload", &digest.to_uppercase()).is_ok());
        assert!(verify_sha256(b"tampered", &digest).is_err());
    }

    #[test]
    fn manifest_loads_from_the_registry_checkout() {
        let home = tempdir().unwrap();
        write_manifest(home.path(), "bench", &manifest_json("bench", &["bench", "proofs"]));
        let manifest = load_manifest(home.path(), "bench").unwrap();
        assert_eq!(manifest.version, "0.1.0");
        assert_eq!(manifest.subcommands, vec!["bench".to_string(), "proofs".to_string()]);
        assert!(manifest.binaries.contains_key("aarch64-apple-darwin"));
    }

    #[test]
    fn manifest_name_must_match_the_requested_extension() {
        let home = tempdir().unwrap();
        write_manifest(home.path(), "bench", &manifest_json("other", &["bench"]));
        assert!(load_manifest(home.path(), "bench").is_err());
    }

    #[test]
    fn missing_manifest_is_an_error() {
        let home = tempdir().unwrap();
        assert!(load_manifest(home.path(), "bench").is_err());
    }

    #[test]
    fn installed_lists_recorded_extensions() {
        let home = tempdir().unwrap();
        record_installed(home.path(), "bench", &["bench"]);
        record_installed(home.path(), "graph", &["graph"]);
        assert_eq!(
            installed(home.path()).unwrap(),
            vec!["bench".to_string(), "graph".to_string()]
        );
    }

    #[test]
    fn owner_of_finds_the_extension_that_provides_a_subcommand() {
        let home = tempdir().unwrap();
        record_installed(home.path(), "bench", &["bench", "proofs"]);
        assert_eq!(owner_of(home.path(), "proofs").unwrap(), Some("bench".to_string()));
        assert_eq!(owner_of(home.path(), "nosuch").unwrap(), None);
    }

    #[test]
    fn team_id_must_match_exactly() {
        let details = "Executable=/x\nIdentifier=bench\nTeamIdentifier=9HSMVVUMVX\n";
        assert!(team_id_matches(details, "9HSMVVUMVX"));
        assert!(!team_id_matches(details, "9HSMVVUMV"));
        assert!(!team_id_matches("Identifier=bench\n", "9HSMVVUMVX"));
    }

    #[test]
    fn unsigned_file_is_rejected() {
        let home = tempdir().unwrap();
        let file = home.path().join("unsigned");
        fs::write(&file, b"not a binary").unwrap();
        assert!(verify_signature(&file).is_err());
    }

    #[test]
    fn apple_signed_binary_from_another_team_is_rejected() {
        assert!(verify_signature(Path::new("/bin/ls")).is_err());
    }

    #[test]
    fn remove_deletes_the_binary_and_its_manifest() {
        let home = tempdir().unwrap();
        record_installed(home.path(), "bench", &["bench"]);
        let dir = bin_dir(home.path());
        fs::create_dir_all(&dir).unwrap();
        fs::write(binary_path(home.path(), "bench"), b"x").unwrap();
        remove(home.path(), "bench").unwrap();
        assert!(installed(home.path()).unwrap().is_empty());
        assert!(!binary_path(home.path(), "bench").exists());
        assert!(remove(home.path(), "bench").is_err());
    }
}
