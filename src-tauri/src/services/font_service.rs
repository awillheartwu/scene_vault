use std::path::Path;

use crate::models::vision::BundledFont;

const FONT_EXTENSIONS: [&str; 4] = ["ttf", "otf", "ttc", "otc"];

/// Lists font files in a directory (sorted by name). The directory is the
/// app-local `fonts` folder where the setup script drops bundled fonts; users
/// can also add their own font files there.
pub fn scan_fonts_dir(directory: &Path) -> Vec<BundledFont> {
    let mut fonts = Vec::new();
    let Ok(entries) = std::fs::read_dir(directory) else {
        return fonts;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
            continue;
        };
        if !FONT_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str()) {
            continue;
        }
        let name = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("font")
            .to_owned();
        fonts.push(BundledFont {
            name,
            path: path.to_string_lossy().into_owned(),
        });
    }
    fonts.sort_by(|first, second| first.name.cmp(&second.name));
    fonts
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn lists_font_files_sorted_and_ignores_others() {
        let workspace = tempdir().expect("tempdir");
        for name in ["b.OTF", "a.ttf", "c.ttc", "readme.txt"] {
            std::fs::write(workspace.path().join(name), b"font").expect("write");
        }
        std::fs::create_dir(workspace.path().join("nested")).expect("nested directory");
        std::fs::write(workspace.path().join("nested").join("d.ttf"), b"font")
            .expect("nested font");

        let fonts = scan_fonts_dir(workspace.path());
        let names: Vec<&str> = fonts.iter().map(|font| font.name.as_str()).collect();
        assert_eq!(names, vec!["a", "b", "c"]);
        assert!(fonts.iter().all(|font| font.path.ends_with("a.ttf")
            || font.path.ends_with("b.OTF")
            || font.path.ends_with("c.ttc")));
    }

    #[test]
    fn missing_directory_returns_empty() {
        assert!(scan_fonts_dir(Path::new("C:\\no-such-fonts-dir")).is_empty());
    }
}
