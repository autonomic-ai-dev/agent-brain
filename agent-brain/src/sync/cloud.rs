//! S3-compatible encrypted cloud sync (local provider for tests).

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use age::secrecy::SecretString;
use age::{Decryptor, Encryptor};
use anyhow::{bail, Context, Result};
use opendal::services::{Fs, S3};
use opendal::Operator;

use crate::db::store::BrainStore;
use crate::secrets;
use crate::settings::CloudSyncSettings;

use super::bundle::{export_bundle, ImportReport, MergePolicy, SyncSource};

#[derive(Debug, Clone, serde::Serialize)]
pub struct CloudSyncStatus {
    pub enabled: bool,
    pub provider: String,
    pub bucket: String,
    pub key: String,
    pub last_push: Option<i64>,
    pub last_pull: Option<i64>,
    pub artifact_present: bool,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct CloudPullReport {
    pub import: ImportReport,
    pub missing_secrets: Vec<String>,
}

pub fn cloud_status(
    home: &Path,
    store: &BrainStore,
    settings: &CloudSyncSettings,
) -> Result<CloudSyncStatus> {
    let artifact_present = artifact_exists(home, settings)?;
    Ok(CloudSyncStatus {
        enabled: settings.enabled,
        provider: settings.provider.clone(),
        bucket: settings.bucket.clone(),
        key: settings.key.clone(),
        last_push: store.cloud_last_push_ms()?,
        last_pull: store.cloud_last_pull_ms()?,
        artifact_present,
    })
}

pub fn cloud_push(store: &BrainStore, home: &Path, settings: &CloudSyncSettings) -> Result<()> {
    validate_settings(settings)?;

    let tmp = tempfile::tempdir().context("create cloud export temp dir")?;
    export_bundle(store, home, Some(tmp.path()))?;
    let packed = pack_dir(tmp.path())?;
    let payload = if settings.encrypt {
        encrypt_blob(&packed, sync_passphrase(settings)?)?
    } else {
        packed
    };

    let op = build_operator(settings)?.blocking();
    op.write(&settings.key, payload)
        .map_err(|e| anyhow::anyhow!("cloud upload failed: {e}"))?;

    store.set_cloud_last_push()?;
    Ok(())
}

pub fn cloud_pull(
    engine: &crate::engine::Engine,
    settings: &CloudSyncSettings,
) -> Result<CloudPullReport> {
    let store = engine.store.as_ref();
    validate_settings(settings)?;

    let op = build_operator(settings)?.blocking();
    let encrypted = op
        .read(&settings.key)
        .map_err(|e| anyhow::anyhow!("cloud download failed: {e}"))?
        .to_vec();

    let packed = if settings.encrypt {
        decrypt_blob(&encrypted, sync_passphrase(settings)?)?
    } else {
        encrypted
    };

    let tmp = tempfile::tempdir().context("create cloud import temp dir")?;
    unpack_dir(&packed, tmp.path())?;

    if !tmp.path().join("manifest.json").is_file() {
        bail!("cloud artifact missing manifest.json");
    }

    let import = engine.import_bundle_queued(
        tmp.path(),
        MergePolicy::NewerWins,
        SyncSource::Cloud,
    )?;

    store.set_cloud_last_pull()?;
    let missing_secrets = secrets::missing_secret_names(store)?;

    if !missing_secrets.is_empty() {
        eprintln!(
            "Missing secrets for upstream MCP: {}",
            missing_secrets.join(", ")
        );
        eprintln!("Run: agent-brain secrets setup");
    }

    Ok(CloudPullReport {
        import,
        missing_secrets,
    })
}

fn validate_settings(settings: &CloudSyncSettings) -> Result<()> {
    if settings.bucket.is_empty() {
        bail!("sync.cloud.bucket is not set in config.yaml");
    }
    if settings.key.is_empty() {
        bail!("sync.cloud.key is not set in config.yaml");
    }
    if settings.encrypt && sync_passphrase(settings).is_err() {
        bail!(
            "{} is not set (required for encrypted cloud sync)",
            settings.encryption_key_env
        );
    }
    Ok(())
}

fn sync_passphrase(settings: &CloudSyncSettings) -> Result<SecretString> {
    let value = std::env::var(&settings.encryption_key_env).with_context(|| {
        format!(
            "read encryption passphrase from env {}",
            settings.encryption_key_env
        )
    })?;
    if value.trim().is_empty() {
        bail!("{} is empty", settings.encryption_key_env);
    }
    Ok(SecretString::from(value))
}

fn build_operator(settings: &CloudSyncSettings) -> Result<Operator> {
    match settings.provider.as_str() {
        "local" => {
            fs::create_dir_all(&settings.bucket).context("create local cloud bucket dir")?;
            let builder = Fs::default().root(&settings.bucket);
            Ok(Operator::new(builder)
                .context("build local cloud operator")?
                .finish())
        }
        "s3" | _ => {
            let mut builder = S3::default().bucket(&settings.bucket);
            if !settings.region.is_empty() {
                builder = builder.region(&settings.region);
            }
            if !settings.endpoint.is_empty() {
                builder = builder.endpoint(&settings.endpoint);
            }
            Ok(Operator::new(builder)
                .context("build s3 cloud operator")?
                .finish())
        }
    }
}

fn artifact_exists(_home: &Path, settings: &CloudSyncSettings) -> Result<bool> {
    if settings.bucket.is_empty() || settings.key.is_empty() {
        return Ok(false);
    }
    if settings.provider == "local" {
        return Ok(PathBuf::from(&settings.bucket).join(&settings.key).is_file());
    }
    match build_operator(settings) {
        Ok(operator) => Ok(operator.blocking().stat(&settings.key).is_ok()),
        Err(_) => Ok(false),
    }
}

fn pack_dir(dir: &Path) -> Result<Vec<u8>> {
    let mut tar_buf = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut tar_buf);
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_file() {
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .context("non-utf8 bundle filename")?;
                let mut file = fs::File::open(&path)?;
                builder.append_file(name, &mut file)?;
            }
        }
        builder.finish()?;
    }
    zstd::encode_all(&tar_buf[..], 3).context("zstd compress bundle")
}

fn unpack_dir(data: &[u8], dest: &Path) -> Result<()> {
    fs::create_dir_all(dest)?;
    let tar_buf = zstd::decode_all(data).context("zstd decompress bundle")?;
    let mut archive = tar::Archive::new(&tar_buf[..]);
    archive.unpack(dest).context("unpack bundle tar")?;
    Ok(())
}

fn encrypt_blob(plaintext: &[u8], passphrase: SecretString) -> Result<Vec<u8>> {
    let encryptor = Encryptor::with_user_passphrase(passphrase);
    let mut encrypted = Vec::new();
    {
        let mut writer = encryptor
            .wrap_output(&mut encrypted)
            .map_err(|e| anyhow::anyhow!("age wrap failed: {e}"))?;
        writer
            .write_all(plaintext)
            .map_err(|e| anyhow::anyhow!("age write failed: {e}"))?;
        writer
            .finish()
            .map_err(|e| anyhow::anyhow!("age finish failed: {e}"))?;
    }
    Ok(encrypted)
}

fn decrypt_blob(ciphertext: &[u8], passphrase: SecretString) -> Result<Vec<u8>> {
    let identity = age::scrypt::Identity::new(passphrase);
    let identities: Vec<Box<dyn age::Identity>> = vec![Box::new(identity)];
    let decryptor = Decryptor::new_buffered(std::io::Cursor::new(ciphertext))
        .map_err(|e| anyhow::anyhow!("age decrypt init failed: {e}"))?;
    let mut reader = decryptor
        .decrypt(identities.iter().map(|id| id.as_ref()))
        .map_err(|e| anyhow::anyhow!("age decrypt failed: {e}"))?;
    let mut plaintext = Vec::new();
    reader
        .read_to_end(&mut plaintext)
        .map_err(|e| anyhow::anyhow!("age read failed: {e}"))?;
    Ok(plaintext)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_unpack_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("hello.txt"), b"world").unwrap();
        let packed = pack_dir(dir.path()).unwrap();
        let out = tempfile::tempdir().unwrap();
        unpack_dir(&packed, out.path()).unwrap();
        assert_eq!(
            fs::read_to_string(out.path().join("hello.txt")).unwrap(),
            "world"
        );
    }

    #[test]
    fn age_round_trip() {
        let secret = SecretString::from("test-passphrase-32-chars-minimum!!".to_string());
        let encrypted = encrypt_blob(b"payload", secret.clone()).unwrap();
        let plain = decrypt_blob(&encrypted, secret).unwrap();
        assert_eq!(plain, b"payload");
    }
}
