use std::{
    collections::{HashMap, HashSet},
    hash::Hash,
    path::{Path, PathBuf},
    time::SystemTime,
};

use crate::services::capture_service;

/// How deep a relink scan walks. Source directories keep game screenshots
/// directly, while archive roots are reorganised by hand, so the two callers
/// need different traversal rules.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ScanMode {
    Flat,
    Recursive,
}

/// One file a relink pass may bind a missing path to.
#[derive(Clone, Debug)]
pub(crate) struct Candidate {
    pub path: PathBuf,
    pub normalized_path: String,
    pub file_name_key: String,
    pub file_size: i64,
    pub modified_at_ms: Option<i64>,
}

/// Outcome of resolving one missing path against scanned candidates. Only a
/// unique match may be written back; guessing would point the database at the
/// wrong file.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Match<'a> {
    Missing,
    Unique(&'a Candidate),
    Ambiguous,
}

#[derive(Debug, Default)]
pub(crate) struct ScanResult {
    pub candidates: Vec<Candidate>,
    /// Roots that could not be resolved or opened at all.
    pub unavailable_roots: u32,
}

/// Identity index of archive-style file names: the short capture identifier
/// embedded in the name, split by main output and avatar output.
pub(crate) type IdentifierIndex = HashMap<(String, bool), Vec<usize>>;

/// Collects image files below the given roots. Symlinks are never followed and
/// system metadata directories are skipped, so pointing a scan at a share root
/// is safe. Shared by source relocation and archive relocation.
pub(crate) async fn scan_roots(roots: &[String], mode: ScanMode) -> ScanResult {
    let mut result = ScanResult::default();
    let mut seen = HashSet::new();
    for root in roots {
        let Ok(canonical_root) = tokio::fs::canonicalize(root).await else {
            result.unavailable_roots = result.unavailable_roots.saturating_add(1);
            continue;
        };
        let mut pending = vec![canonical_root.clone()];
        let mut root_opened = false;
        while let Some(directory) = pending.pop() {
            let Ok(mut entries) = tokio::fs::read_dir(&directory).await else {
                continue;
            };
            root_opened = true;
            loop {
                let Ok(next) = entries.next_entry().await else {
                    break;
                };
                let Some(entry) = next else { break };
                let path = entry.path();
                if mode == ScanMode::Recursive {
                    // Directories carry no image suffix, so the entry type has
                    // to be probed before the supported-image filter.
                    let Ok(file_type) = entry.file_type().await else {
                        continue;
                    };
                    if file_type.is_symlink() {
                        continue;
                    }
                    if file_type.is_dir() {
                        if !is_skipped_directory(&path) {
                            pending.push(path);
                        }
                        continue;
                    }
                    if !file_type.is_file() {
                        continue;
                    }
                }
                if !capture_service::is_supported_image(&path) {
                    continue;
                }
                let Ok(metadata) = entry.metadata().await else {
                    continue;
                };
                if !metadata.is_file() || metadata.len() == 0 {
                    continue;
                }
                let Ok(canonical_path) = tokio::fs::canonicalize(&path).await else {
                    continue;
                };
                if !capture_service::path_is_within(&canonical_path, &canonical_root) {
                    continue;
                }
                let normalized_path = capture_service::normalized_path_string(&canonical_path);
                if !seen.insert(normalized_path.clone()) {
                    continue;
                }
                let Some(file_name_key) = canonical_path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .map(str::to_lowercase)
                else {
                    continue;
                };
                result.candidates.push(Candidate {
                    path: canonical_path,
                    normalized_path,
                    file_name_key,
                    file_size: i64::try_from(metadata.len()).unwrap_or(i64::MAX),
                    modified_at_ms: metadata.modified().ok().and_then(unix_millis),
                });
            }
        }
        if !root_opened {
            result.unavailable_roots = result.unavailable_roots.saturating_add(1);
        }
    }
    result
        .candidates
        .sort_by(|left, right| left.normalized_path.cmp(&right.normalized_path));
    result
}

/// Directories no scan descends into: NAS metadata, recycle bins and hidden
/// staging directories.
fn is_skipped_directory(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
        return true;
    };
    if name.is_empty() || name.starts_with('@') || name.starts_with('#') || name.starts_with('.') {
        return true;
    }
    name.eq_ignore_ascii_case("$RECYCLE.BIN")
        || name.eq_ignore_ascii_case("System Volume Information")
}

/// Every delimited hexadecimal run of at least six characters is treated as a
/// candidate identifier: Scene Vault appends the short capture id to archive
/// names, and later processing passes may append their own markers.
pub(crate) fn identifier_tokens(stem: &str) -> Vec<String> {
    stem.split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|part| {
            (6..=16).contains(&part.len()) && part.chars().all(|value| value.is_ascii_hexdigit())
        })
        .map(|part| part.to_ascii_lowercase())
        .collect()
}

/// Splits an archive file stem into its base stem and whether it names an
/// avatar output.
pub(crate) fn split_avatar_stem(stem: &str) -> (&str, bool) {
    let lower = stem.to_ascii_lowercase();
    match lower.strip_suffix("-avatar") {
        Some(value) => (&stem[..value.len()], true),
        None => (stem, false),
    }
}

pub(crate) fn index_by_identifier(candidates: &[Candidate]) -> IdentifierIndex {
    let mut index: IdentifierIndex = HashMap::new();
    for (position, candidate) in candidates.iter().enumerate() {
        let Some(stem) = candidate
            .path
            .file_stem()
            .and_then(|value| value.to_str())
        else {
            continue;
        };
        let (stem, is_avatar) = split_avatar_stem(stem);
        for token in identifier_tokens(stem) {
            index.entry((token, is_avatar)).or_default().push(position);
        }
    }
    index
}

/// Resolves one key against a prebuilt index; ambiguity is reported instead of
/// picking a winner.
pub(crate) fn resolve<'a, K>(
    index: &HashMap<K, Vec<usize>>,
    candidates: &'a [Candidate],
    key: &K,
) -> Match<'a>
where
    K: Eq + Hash,
{
    let matches = index.get(key).map(Vec::as_slice).unwrap_or(&[]);
    match matches.len() {
        0 => Match::Missing,
        1 => Match::Unique(&candidates[matches[0]]),
        _ => Match::Ambiguous,
    }
}

pub(crate) fn unix_millis(time: SystemTime) -> Option<i64> {
    time.duration_since(SystemTime::UNIX_EPOCH)
        .ok()
        .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
}
