use anyhow::{Context, Result};
use hunkset::OccurrenceId;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;
use walkdir::WalkDir;

pub const ID_PREFIX: &str = "hunk-";

#[derive(Clone, Debug)]
pub struct ComparisonFingerprint([u8; 32]);

#[cfg(test)]
impl ComparisonFingerprint {
    pub fn test(value: u8) -> Self {
        Self([value; 32])
    }
}

pub struct FileIdentity<'a> {
    pub comparison: &'a ComparisonFingerprint,
    pub old_path: Option<&'a str>,
    pub new_path: Option<&'a str>,
    pub before: &'a [u8],
    pub after: &'a [u8],
    pub before_executable: bool,
    pub after_executable: bool,
}

pub fn comparison_fingerprint(
    before_root: &Path,
    after_root: &Path,
) -> Result<ComparisonFingerprint> {
    #[cfg(not(unix))]
    anyhow::bail!("occurrence IDs require executable-mode inspection on this platform");

    let mut encoder = CanonicalEncoder::new(b"jj-hunk-comparison-v1");
    encode_manifest(&mut encoder, b"before", before_root)?;
    encode_manifest(&mut encoder, b"after", after_root)?;
    Ok(ComparisonFingerprint(encoder.finish()))
}

pub fn text_id(
    identity: &FileIdentity<'_>,
    kind: &str,
    before_start: usize,
    before_length: usize,
    after_start: usize,
    after_length: usize,
    removed: &str,
    added: &str,
) -> OccurrenceId {
    let mut encoder = occurrence_encoder(identity, b"text");
    encoder.field(b"kind", kind.as_bytes());
    encoder.usize_field(b"before_start", before_start);
    encoder.usize_field(b"before_length", before_length);
    encoder.usize_field(b"after_start", after_start);
    encoder.usize_field(b"after_length", after_length);
    encoder.field(b"removed", removed.as_bytes());
    encoder.field(b"added", added.as_bytes());
    OccurrenceId::from_sha256(encoder.finish())
}

pub fn file_id(identity: &FileIdentity<'_>, kind: &str) -> OccurrenceId {
    let mut encoder = occurrence_encoder(identity, b"file");
    encoder.field(b"kind", kind.as_bytes());
    OccurrenceId::from_sha256(encoder.finish())
}

fn occurrence_encoder(identity: &FileIdentity<'_>, category: &[u8]) -> CanonicalEncoder {
    let mut encoder = CanonicalEncoder::new(b"jj-hunk-occurrence-v1");
    encoder.field(b"comparison", &identity.comparison.0);
    encoder.field(b"category", category);
    encoder.optional_field(b"old_path", identity.old_path.map(str::as_bytes));
    encoder.optional_field(b"new_path", identity.new_path.map(str::as_bytes));
    encoder.field(b"before", identity.before);
    encoder.field(b"after", identity.after);
    encoder.bool_field(b"before_executable", identity.before_executable);
    encoder.bool_field(b"after_executable", identity.after_executable);
    encoder
}

fn encode_manifest(encoder: &mut CanonicalEncoder, side: &[u8], root: &Path) -> Result<()> {
    let entries = WalkDir::new(root)
        .min_depth(1)
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;
    let mut entries = entries
        .into_iter()
        .map(|entry| Ok((relative_path_bytes(root, entry.path())?, entry)))
        .collect::<Result<Vec<_>>>()?;
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    for (path, entry) in entries {
        if path == b"JJ-INSTRUCTIONS" {
            continue;
        }
        let metadata = fs::symlink_metadata(entry.path())?;
        encoder.field(b"side", side);
        encoder.field(b"path", &path);
        if metadata.file_type().is_file() {
            encoder.field(b"type", b"file");
            encoder.bool_field(b"executable", executable(&metadata));
            encoder.field(
                b"content",
                &fs::read(entry.path()).with_context(|| {
                    format!(
                        "Failed to read materialized file {}",
                        entry.path().display()
                    )
                })?,
            );
        } else if metadata.file_type().is_dir() {
            encoder.field(b"type", b"directory");
        } else if metadata.file_type().is_symlink() {
            encoder.field(b"type", b"symlink");
            let target = fs::read_link(entry.path())?;
            encoder.field(b"target", &path_bytes(&target)?);
        } else {
            encoder.field(b"type", b"special");
        }
    }
    Ok(())
}

fn relative_path_bytes(root: &Path, path: &Path) -> Result<Vec<u8>> {
    path_bytes(path.strip_prefix(root).unwrap_or(path))
}

#[cfg(unix)]
fn path_bytes(path: &Path) -> Result<Vec<u8>> {
    use std::os::unix::ffi::OsStrExt;
    Ok(path.as_os_str().as_bytes().to_vec())
}

#[cfg(not(unix))]
fn path_bytes(path: &Path) -> Result<Vec<u8>> {
    path.to_str()
        .map(|value| value.replace('\\', "/").into_bytes())
        .ok_or_else(|| {
            anyhow::anyhow!("A materialized path is not valid UTF-8: {}", path.display())
        })
}

#[cfg(unix)]
fn executable(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn executable(_metadata: &fs::Metadata) -> bool {
    false
}

struct CanonicalEncoder(Sha256);

impl CanonicalEncoder {
    fn new(domain: &[u8]) -> Self {
        let mut encoder = Self(Sha256::new());
        encoder.field(b"domain", domain);
        encoder
    }

    fn field(&mut self, tag: &[u8], value: &[u8]) {
        self.0.update((tag.len() as u64).to_be_bytes());
        self.0.update(tag);
        self.0.update((value.len() as u64).to_be_bytes());
        self.0.update(value);
    }

    fn optional_field(&mut self, tag: &[u8], value: Option<&[u8]>) {
        let mut encoded = Vec::with_capacity(value.map_or(1, |value| value.len() + 1));
        encoded.push(u8::from(value.is_some()));
        if let Some(value) = value {
            encoded.extend_from_slice(value);
        }
        self.field(tag, &encoded);
    }

    fn bool_field(&mut self, tag: &[u8], value: bool) {
        self.field(tag, &[u8::from(value)]);
    }

    fn usize_field(&mut self, tag: &[u8], value: usize) {
        self.field(tag, &(value as u64).to_be_bytes());
    }

    fn finish(self) -> [u8; 32] {
        self.0.finalize().into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comparison_uses_contents_existence_modes_and_ignores_incidental_metadata() {
        let roots = tempfile::tempdir().unwrap();
        let before = roots.path().join("before");
        let after = roots.path().join("after");
        fs::create_dir_all(&before).unwrap();
        fs::create_dir_all(&after).unwrap();
        fs::write(before.join("item"), b"").unwrap();
        fs::write(after.join("item"), b"").unwrap();
        let stable = comparison_fingerprint(&before, &after).unwrap();

        fs::write(after.join("JJ-INSTRUCTIONS"), b"generated text").unwrap();
        let with_instructions = comparison_fingerprint(&before, &after).unwrap();
        assert_eq!(stable.0, with_instructions.0);

        let mut permissions = fs::metadata(after.join("item")).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(after.join("item"), permissions).unwrap();
        let readonly = comparison_fingerprint(&before, &after).unwrap();
        assert_eq!(stable.0, readonly.0);

        fs::remove_file(after.join("item")).unwrap();
        let missing = comparison_fingerprint(&before, &after).unwrap();
        assert_ne!(stable.0, missing.0);
    }

    #[cfg(unix)]
    #[test]
    fn comparison_uses_executable_state() {
        use std::os::unix::fs::PermissionsExt;
        let roots = tempfile::tempdir().unwrap();
        let before = roots.path().join("before");
        let after = roots.path().join("after");
        fs::create_dir_all(&before).unwrap();
        fs::create_dir_all(&after).unwrap();
        fs::write(before.join("script"), b"same").unwrap();
        fs::write(after.join("script"), b"same").unwrap();
        let regular = comparison_fingerprint(&before, &after).unwrap();
        fs::set_permissions(after.join("script"), fs::Permissions::from_mode(0o755)).unwrap();
        let executable = comparison_fingerprint(&before, &after).unwrap();
        assert_ne!(regular.0, executable.0);
    }
}
