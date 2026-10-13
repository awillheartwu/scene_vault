use std::path::{Path, PathBuf};

use crate::{error::AppError, models::engine::EngineDetection};

const ENGINE_GAME_DIR: &str = "game";

/// Detects a Ren'Py build from a user-selected game folder or executable.
/// Read-only: nothing is written and the game is never launched.
pub async fn detect(path: &Path) -> Result<EngineDetection, AppError> {
    let canonical = tokio::fs::canonicalize(path)
        .await
        .map_err(|error| AppError::Validation(format!("无法访问所选路径：{}", error)))?;
    let start = canonical
        .parent()
        .map(Path::to_path_buf)
        .filter(|_| canonical.is_file())
        .unwrap_or_else(|| canonical.clone());

    let engine_root = match resolve_engine_root(&start).await {
        Some(root) => root,
        None => {
            return Err(AppError::Validation(
                "所选目录看起来不是 Ren'Py 游戏：缺少 game/ 目录或 renpy/ 运行库".to_owned(),
            ))
        }
    };
    // The actual game directory is the ownership key: resolve symlinks and
    // junctions here so two differently spelled roots cannot claim the same
    // directory as two separate installs.
    let game_directory = tokio::fs::canonicalize(engine_root.join(ENGINE_GAME_DIR))
        .await
        .map_err(|error| AppError::Validation(format!("无法访问 game 目录：{error}")))?;
    let python_major = detect_python_major(&engine_root).await;
    let engine_version = detect_engine_version(&engine_root, python_major).await;
    let bridge_installed = tokio::fs::metadata(
        game_directory.join(crate::services::renpy_bridge_service::BRIDGE_FILE_NAME),
    )
    .await
    .is_ok_and(|metadata| metadata.is_file());

    Ok(EngineDetection {
        engine: crate::models::engine::ENGINE_RENPY.to_owned(),
        engine_root: crate::services::capture_service::path_to_string(&engine_root),
        game_directory: crate::services::capture_service::path_to_string(&game_directory),
        python_major,
        engine_version,
        bridge_installed,
    })
}

/// Reads version constants without executing Python. Older builds keep the
/// tuple in `renpy/__init__.py` and only the revision in `vc_version.py`.
pub(crate) async fn detect_engine_version(
    root: &Path,
    python_major: Option<i64>,
) -> Option<String> {
    let renpy = root.join("renpy");
    let text = tokio::fs::read_to_string(renpy.join("vc_version.py"))
        .await
        .ok();
    for rest in python_assignment_values(text.as_deref().unwrap_or_default(), "version") {
        let rest = match rest.chars().next() {
            Some('u') | Some('U') => rest[1..].trim_start(),
            _ => rest,
        };
        let Some(quote) = rest.chars().next().filter(|c| *c == '\'' || *c == '"') else {
            continue;
        };
        let inner = &rest[1..];
        let Some(end) = inner.find(quote) else {
            continue;
        };
        let value = inner[..end].trim();
        if !value.is_empty() && !value.contains('\\') && inner[end + 1..].trim().is_empty() {
            return Some(value.to_owned());
        }
    }

    let init = tokio::fs::read_to_string(renpy.join("__init__.py"))
        .await
        .ok()?;
    let revision = text.as_deref().and_then(|text| {
        let mut values = python_assignment_values(text, "vc_version");
        let value = parse_version_number(values.next()?)?;
        values.next().is_none().then_some(value)
    });
    parse_legacy_engine_version(&init, revision, python_major)
}

/// Ignores comments, quoted source examples and similarly prefixed names.
fn python_assignment_values<'a>(text: &'a str, name: &'a str) -> impl Iterator<Item = &'a str> {
    let mut quote: Option<(u8, usize)> = None;
    text.lines().filter_map(move |line| {
        let inside_string = quote.is_some();
        let bytes = line.as_bytes();
        let mut index = 0;
        while index < bytes.len() {
            match quote {
                Some(_) if bytes[index] == b'\\' => index += 2,
                Some((character, length)) => {
                    if bytes[index..].starts_with(&[character; 3][..length]) {
                        quote = None;
                        index += length;
                    } else {
                        index += 1;
                    }
                }
                None if bytes[index] == b'#' => break,
                None if bytes[index] == b'\'' || bytes[index] == b'"' => {
                    let character = bytes[index];
                    let length = if bytes[index..].starts_with(&[character; 3]) {
                        3
                    } else {
                        1
                    };
                    quote = Some((character, length));
                    index += length;
                }
                None => index += 1,
            }
        }
        if inside_string || quote.is_some() {
            return None;
        }
        let (target, value) = line[..index.min(bytes.len())].split_once('=')?;
        (target.trim() == name).then_some(value.trim())
    })
}

fn parse_version_number(value: &str) -> Option<u64> {
    let value = value.trim();
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    // A leading zero may be an octal literal in Python 2.
    if value.len() > 1 && value.starts_with('0') {
        return None;
    }
    value.parse().ok()
}

fn parse_legacy_engine_version(
    text: &str,
    revision: Option<u64>,
    python_major: Option<i64>,
) -> Option<String> {
    let versions = python_assignment_values(text, "version_tuple")
        .map(|value| {
            let value = value.strip_prefix("VersionTuple").unwrap_or(value).trim();
            let inner = value.strip_prefix('(')?.strip_suffix(')')?.trim();
            let inner = inner.strip_suffix(',').unwrap_or(inner);
            let mut fields = inner.split(',').map(str::trim);
            let major = parse_version_number(fields.next()?)?;
            let minor = parse_version_number(fields.next()?)?;
            let patch = parse_version_number(fields.next()?)?;
            let build = match fields.next()? {
                "vc_version" => revision?,
                value => parse_version_number(value)?,
            };
            fields
                .next()
                .is_none()
                .then_some([major, minor, patch, build])
        })
        .collect::<Option<Vec<_>>>()?;
    let version = match versions.as_slice() {
        [version] => version,
        _ => {
            // Dual-version sources select Ren'Py 7 on Python 2 and 8 on Python 3.
            // Without a known runtime or a unique matching tuple, do not guess.
            let major = match python_major? {
                2 => 7,
                3 => 8,
                _ => return None,
            };
            let mut matching = versions.iter().filter(|version| version[0] == major);
            let version = matching.next()?;
            if matching.next().is_some() {
                return None;
            }
            version
        }
    };
    Some(format!(
        "{}.{}.{}.{}",
        version[0], version[1], version[2], version[3]
    ))
}

/// Accepts the game root, the `game/` folder itself, or a subfolder of a
/// game root. Returns the folder that owns `game/`.
async fn resolve_engine_root(start: &Path) -> Option<PathBuf> {
    if is_engine_root(start).await {
        return Some(start.to_path_buf());
    }
    if start
        .file_name()
        .is_some_and(|name| name == ENGINE_GAME_DIR)
    {
        if let Some(parent) = start.parent() {
            if is_engine_root(parent).await {
                return Some(parent.to_path_buf());
            }
        }
    }
    if let Some(parent) = start.parent() {
        if is_engine_root(parent).await {
            return Some(parent.to_path_buf());
        }
    }
    None
}

async fn is_engine_root(root: &Path) -> bool {
    let game_directory = root.join(ENGINE_GAME_DIR);
    if !tokio::fs::metadata(&game_directory)
        .await
        .is_ok_and(|metadata| metadata.is_dir())
    {
        return false;
    }
    if tokio::fs::metadata(root.join("renpy"))
        .await
        .is_ok_and(|metadata| metadata.is_dir())
    {
        return true;
    }
    // Some repacks drop renpy/ but keep compiled scripts or archives.
    let mut entries = match tokio::fs::read_dir(&game_directory).await {
        Ok(entries) => entries,
        Err(_) => return false,
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name().to_string_lossy().to_lowercase();
        if name.ends_with(".rpyc") || name.ends_with(".rpa") || name.ends_with(".rpy") {
            return true;
        }
    }
    false
}

/// Reads the bundled Python major version from `lib/`. Windows builds are
/// preferred when a game ships several platform runtimes.
async fn detect_python_major(root: &Path) -> Option<i64> {
    let lib = root.join("lib");
    let mut entries = tokio::fs::read_dir(&lib).await.ok()?;
    let mut windows: Option<i64> = None;
    let mut any: Option<i64> = None;
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name().to_string_lossy().to_lowercase();
        let major = if name.starts_with("py3-") {
            3
        } else if name.starts_with("py2-") {
            2
        } else {
            continue;
        };
        if any.map(|current| major > current).unwrap_or(true) {
            any = Some(major);
        }
        if name.contains("windows") {
            windows = Some(major);
        }
    }
    windows.or(any)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    async fn write_minimal_game(root: &Path, python: &str) {
        tokio::fs::create_dir_all(root.join("game")).await.unwrap();
        tokio::fs::write(root.join("game/script.rpyc"), b"compiled")
            .await
            .unwrap();
        tokio::fs::create_dir_all(root.join(format!("lib/{python}")))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn detects_engine_root_and_python_major() {
        let workspace = tempdir().unwrap();
        let root = workspace.path().join("demo");
        write_minimal_game(&root, "py3-windows-x86_64").await;

        let detection = detect(&root).await.expect("detect");
        assert_eq!(detection.engine, "renpy");
        assert_eq!(detection.python_major, Some(3));
        assert!(!detection.bridge_installed);
    }

    #[tokio::test]
    async fn reads_the_exact_engine_version_from_vc_version_py() {
        let workspace = tempdir().unwrap();
        let root = workspace.path().join("demo");
        write_minimal_game(&root, "py3-windows-x86_64").await;
        tokio::fs::create_dir_all(root.join("renpy")).await.unwrap();
        tokio::fs::write(
            root.join("renpy/__init__.py"),
            "version_tuple = (7, 3, 2, vc_version)\n",
        )
        .await
        .unwrap();
        tokio::fs::write(
            root.join("renpy/vc_version.py"),
            "branch = 'fix'\nvc_version = 320\nversion = '8.1.3.23091805'\nversion_name = 'x'\n",
        )
        .await
        .unwrap();
        let detection = detect(&root).await.expect("detect");
        assert_eq!(detection.engine_version.as_deref(), Some("8.1.3.23091805"));

        // Python 2 games spell the same line with a u prefix.
        tokio::fs::write(
            root.join("renpy/vc_version.py"),
            "version = u'7.6.1.23060707'\n",
        )
        .await
        .unwrap();
        let detection = detect(&root).await.expect("detect py2");
        assert_eq!(detection.engine_version.as_deref(), Some("7.6.1.23060707"));

        // A repack without the file simply reports no version.
        tokio::fs::remove_file(root.join("renpy/vc_version.py"))
            .await
            .unwrap();
        let detection = detect(&root).await.expect("detect without version");
        assert_eq!(detection.engine_version, None);
    }

    #[tokio::test]
    async fn reads_legacy_engine_version_with_a_revision_only_module() {
        let workspace = tempdir().unwrap();
        let root = workspace.path().join("wiab");
        // Old bundles do not necessarily have a py2-prefixed runtime directory.
        write_minimal_game(&root, "windows-i686").await;
        tokio::fs::create_dir_all(root.join("renpy")).await.unwrap();
        tokio::fs::write(
            root.join("renpy/__init__.py"),
            "try:\n    from renpy.vc_version import vc_version; vc_version\n\
             except ImportError:\n    vc_version = 0\n\
             version_tuple = (7, 3, 2, vc_version)\n",
        )
        .await
        .unwrap();
        tokio::fs::write(root.join("renpy/vc_version.py"), "vc_version = 320")
            .await
            .unwrap();

        let detection = detect(&root).await.expect("detect legacy game");
        assert_eq!(detection.python_major, None);
        assert_eq!(detection.engine_version.as_deref(), Some("7.3.2.320"));

        // Do not substitute __init__.py's import-error fallback for a missing revision.
        tokio::fs::remove_file(root.join("renpy/vc_version.py"))
            .await
            .unwrap();
        assert_eq!(detect_engine_version(&root, None).await, None);
    }

    #[tokio::test]
    async fn ignores_commented_quoted_and_prefixed_version_assignments() {
        let workspace = tempdir().unwrap();
        let root = workspace.path();
        tokio::fs::create_dir_all(root.join("renpy")).await.unwrap();
        tokio::fs::write(
            root.join("renpy/vc_version.py"),
            r#"# version = '9.9.9.999'
version_name = '9.9.9.999'
previous_version = '9.9.9.999'
example = """
version = '9.9.9.999'
"""
version = U"7.6.1.23060707"  # actual version
"#,
        )
        .await
        .unwrap();
        assert_eq!(
            detect_engine_version(root, None).await.as_deref(),
            Some("7.6.1.23060707")
        );

        tokio::fs::write(
            root.join("renpy/__init__.py"),
            "version_tuple = VersionTuple(7, 3, 2, vc_version)\n",
        )
        .await
        .unwrap();
        tokio::fs::write(
            root.join("renpy/vc_version.py"),
            "# vc_version = 999\nvc_version_extra = 999\nvc_version = 320 # build\n",
        )
        .await
        .unwrap();
        assert_eq!(
            detect_engine_version(root, None).await.as_deref(),
            Some("7.3.2.320")
        );

        for text in [
            "vc_version = '320'",
            "vc_version = -320",
            "vc_version = 0320",
            "vc_version = 320 + 1",
            "vc_version = 18446744073709551616",
            "vc_version = 320\nvc_version = 321",
            "version = '8.1.3.23091805' + suffix",
            "version = '8.1.3.23091805",
            "version = b'8.1.3.23091805'",
            r"version = '8.1.3.23091805\'",
        ] {
            tokio::fs::write(root.join("renpy/vc_version.py"), text)
                .await
                .unwrap();
            assert_eq!(detect_engine_version(root, None).await, None, "{text}");
        }
    }

    #[test]
    fn reads_plain_and_named_version_tuples_without_a_runtime() {
        let text = r#"# version_tuple = (9, 9, 9, 999)
version_tuple_backup = (9, 9, 9, 999)
example = '''
version_tuple = (9, 9, 9, 999)
'''
version_tuple = (7, 3, 2, vc_version) # actual tuple
"#;
        assert_eq!(
            parse_legacy_engine_version(text, Some(320), None).as_deref(),
            Some("7.3.2.320")
        );
        assert_eq!(
            parse_legacy_engine_version(
                "version_tuple = VersionTuple(8, 0, 3, 22090809,)",
                None,
                None,
            )
            .as_deref(),
            Some("8.0.3.22090809")
        );
    }

    #[test]
    fn requires_a_known_runtime_to_select_a_unique_dual_version_tuple() {
        let text = "if sys.version_info.major == 3:
    version_tuple = VersionTuple(8, 0, 3, vc_version)
else:
    version_tuple = VersionTuple(7, 5, 3, vc_version)
";
        assert_eq!(
            parse_legacy_engine_version(text, Some(22090809), None),
            None
        );
        assert_eq!(
            parse_legacy_engine_version(text, Some(22090809), Some(2)).as_deref(),
            Some("7.5.3.22090809")
        );
        assert_eq!(
            parse_legacy_engine_version(text, Some(22090809), Some(3)).as_deref(),
            Some("8.0.3.22090809")
        );
        assert_eq!(
            parse_legacy_engine_version(text, Some(22090809), Some(4)),
            None
        );
        assert_eq!(parse_legacy_engine_version(text, None, Some(3)), None);

        let ambiguous = format!("{text}version_tuple = (8, 1, 0, vc_version)\n");
        assert_eq!(
            parse_legacy_engine_version(&ambiguous, Some(22090809), Some(3)),
            None
        );
        let same_major = "version_tuple = (7, 3, 2, 320)\n\
                          version_tuple = (7, 4, 0, 321)\n";
        assert_eq!(parse_legacy_engine_version(same_major, None, Some(2)), None);
        assert_eq!(parse_legacy_engine_version(same_major, None, Some(3)), None);
    }

    #[test]
    fn rejects_malformed_or_dynamic_version_tuples() {
        for text in [
            "# version_tuple = (7, 3, 2, 320)",
            "old_version_tuple = (7, 3, 2, 320)",
            "version_tuple = (7, 3, 2)",
            "version_tuple = (7, 3, 2, 320, 1)",
            "version_tuple = (7, 3, 2, 320,,)",
            "version_tuple = (7, 3, 2, vc_version_extra)",
            "version_tuple = (7, 3, 2, -320)",
            "version_tuple = (7, 3.0, 2, 320)",
            "version_tuple = (07, 3, 2, 320)",
            "version_tuple = (7, 3, 2, 320 + 1)",
            "version_tuple = (7, 3, 2, 18446744073709551616)",
            "version_tuple = (7, 3, 2, 320",
            "version_tuple = [7, 3, 2, 320]",
            "version_tuple = '(7, 3, 2, 320)'",
            "version_tuple == (7, 3, 2, 320)",
            "version_tuple = VersionTupleExtra(7, 3, 2, 320)",
            "version_tuple = VersionTuple(major=7, minor=3, patch=2, commit=320)",
            "version_tuple = (7, 3, 2, 320) if enabled else (8, 0, 0, 321)",
            "version_tuple = (7, 3, 2, 320)\nversion_tuple = (8, 0, 0, missing)",
        ] {
            assert_eq!(
                parse_legacy_engine_version(text, Some(320), Some(2)),
                None,
                "{text}"
            );
        }
    }

    #[tokio::test]
    async fn accepts_the_game_folder_itself() {
        let workspace = tempdir().unwrap();
        let root = workspace.path().join("demo");
        write_minimal_game(&root, "py2-windows-x86_64").await;
        tokio::fs::create_dir_all(root.join("renpy")).await.unwrap();

        let detection = detect(&root.join("game"))
            .await
            .expect("detect from game dir");
        assert_eq!(detection.python_major, Some(2));
        // `path_to_string` normalizes the `\\?\` prefix canonicalize adds.
        let canonical = tokio::fs::canonicalize(&root).await.unwrap();
        assert_eq!(
            detection.engine_root,
            crate::services::capture_service::path_to_string(&canonical)
        );
    }

    #[tokio::test]
    async fn reports_bridge_presence() {
        let workspace = tempdir().unwrap();
        let root = workspace.path().join("demo");
        write_minimal_game(&root, "py3-windows-x86_64").await;
        tokio::fs::write(
            root.join("game")
                .join(crate::services::renpy_bridge_service::BRIDGE_FILE_NAME),
            b"bridge",
        )
        .await
        .unwrap();

        let detection = detect(&root).await.expect("detect");
        assert!(detection.bridge_installed);
    }

    #[tokio::test]
    async fn rejects_directories_without_game_data() {
        let workspace = tempdir().unwrap();
        let root = workspace.path().join("plain");
        tokio::fs::create_dir_all(&root).await.unwrap();
        let error = detect(&root).await.expect_err("not a renpy game");
        assert!(matches!(error, AppError::Validation(_)));
    }
}
