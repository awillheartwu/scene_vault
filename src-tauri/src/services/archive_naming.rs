use crate::{error::AppError, models::archive_naming::ArchiveNamingSettings};

/// Placeholders supported by the archive naming template. Rendering happens
/// after every value has been sanitized for Windows compatibility; the
/// assembled name is sanitized once more at the end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Placeholder {
    Source,
    Character,
    Date,
    Time,
    DateTime,
    Id,
    Classification,
    Seq,
}

impl Placeholder {
    fn token(self) -> &'static str {
        match self {
            Placeholder::Source => "{source}",
            Placeholder::Character => "{character}",
            Placeholder::Date => "{date}",
            Placeholder::Time => "{time}",
            Placeholder::DateTime => "{datetime}",
            Placeholder::Id => "{id}",
            Placeholder::Classification => "{classification}",
            Placeholder::Seq => "{seq}",
        }
    }
}

const PLACEHOLDERS: [Placeholder; 8] = [
    Placeholder::Source,
    Placeholder::Character,
    Placeholder::Date,
    Placeholder::Time,
    Placeholder::DateTime,
    Placeholder::Id,
    Placeholder::Classification,
    Placeholder::Seq,
];

#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment {
    Text(String),
    Placeholder(Placeholder),
}

#[derive(Debug, Clone)]
struct Template {
    segments: Vec<Segment>,
    has_id: bool,
    has_seq: bool,
}

impl Template {
    fn parse(template: &str) -> Result<Self, AppError> {
        if template.trim().is_empty() {
            return Err(AppError::Validation(
                "archive naming template cannot be empty".to_owned(),
            ));
        }
        let mut segments = Vec::new();
        let mut text = String::new();
        let mut has_id = false;
        let mut has_seq = false;
        let mut rest = template;
        while !rest.is_empty() {
            match rest.find('{') {
                None => {
                    text.push_str(rest);
                    rest = "";
                }
                Some(open) => {
                    text.push_str(&rest[..open]);
                    let after = &rest[open..];
                    let known = PLACEHOLDERS
                        .iter()
                        .find(|placeholder| after.starts_with(placeholder.token()));
                    match known {
                        Some(placeholder) => {
                            if !text.is_empty() {
                                segments.push(Segment::Text(std::mem::take(&mut text)));
                            }
                            segments.push(Segment::Placeholder(*placeholder));
                            has_id |= *placeholder == Placeholder::Id;
                            has_seq |= *placeholder == Placeholder::Seq;
                            rest = &after[placeholder.token().len()..];
                        }
                        None => {
                            // An unrecognized "{...}" group is a hard error so
                            // typos do not silently produce odd file names.
                            let close = after.find('}').map(|index| open + index + 1);
                            let token = close
                                .map(|end| &template[open..end])
                                .unwrap_or(&template[open..]);
                            return Err(AppError::Validation(format!(
                                "unknown archive naming placeholder {token}"
                            )));
                        }
                    }
                }
            }
        }
        if !text.is_empty() {
            segments.push(Segment::Text(text));
        }
        Ok(Self {
            segments,
            has_id,
            has_seq,
        })
    }
}

/// Context values available while rendering an archive file name.
#[derive(Debug, Clone, Default)]
pub struct ArchiveNameContext<'a> {
    /// Sanitized source file stem (without extension).
    pub source_stem: &'a str,
    /// Optional character display name; sanitized before rendering.
    pub character_name: Option<&'a str>,
    /// Capture item id; rendered through `{id}` as a short identifier.
    pub capture_item_id: &'a str,
    /// Classification value: `person`, `scene` or `private`.
    pub classification: &'a str,
    /// UTC RFC 3339 capture timestamp used by `{date}`/`{time}`/`{datetime}`.
    pub captured_at: Option<&'a str>,
    /// Renders `-avatar` at the end of the assembled name.
    pub avatar: bool,
}

/// Validates a naming template and separator before persisting them.
pub fn validate(settings: &ArchiveNamingSettings) -> Result<(), AppError> {
    Template::parse(&settings.template)?;
    if settings.separator.chars().any(char::is_control) {
        return Err(AppError::Validation(
            "archive naming separator cannot contain control characters".to_owned(),
        ));
    }
    Ok(())
}

/// Whether the template contains the `{seq}` placeholder. The archive flow
/// uses this to decide whether it should bump the sequence number when the
/// rendered destination already exists with different content.
pub fn uses_seq(settings: &ArchiveNamingSettings) -> bool {
    Template::parse(&settings.template)
        .map(|template| template.has_seq)
        .unwrap_or(false)
}

/// Renders the configured template for one capture. When the template contains
/// neither `{id}` nor `{seq}`, a short capture identifier is appended through
/// the configured separator so every archive file name stays unique.
pub fn render(settings: &ArchiveNamingSettings, context: &ArchiveNameContext) -> String {
    render_with_seq(settings, context, 1)
}

/// Renders a name with an explicit sequence number for `{seq}`.
pub fn render_with_seq(
    settings: &ArchiveNamingSettings,
    context: &ArchiveNameContext,
    seq: u32,
) -> String {
    let template = Template::parse(&settings.template)
        .unwrap_or_else(|_| Template::parse("{source} - {character} - {id}").expect("default"));
    let mut raw = render_segments(&template, context, seq);
    if !template.has_id && !template.has_seq {
        raw.push_str(&settings.separator);
        raw.push_str(&short_identifier(context.capture_item_id));
    }
    if context.avatar {
        raw.push_str("-avatar");
    }
    let collapsed = collapse_repeated_separators(&raw, &settings.separator);
    sanitize_windows_component(&collapsed)
}

fn render_segments(template: &Template, context: &ArchiveNameContext, seq: u32) -> String {
    let mut rendered = String::new();
    for segment in &template.segments {
        match segment {
            Segment::Text(text) => rendered.push_str(text),
            Segment::Placeholder(placeholder) => {
                rendered.push_str(&placeholder_value(*placeholder, context, seq));
            }
        }
    }
    rendered
}

fn placeholder_value(placeholder: Placeholder, context: &ArchiveNameContext, seq: u32) -> String {
    match placeholder {
        Placeholder::Source => sanitize_windows_component(context.source_stem),
        Placeholder::Character => context
            .character_name
            .map(sanitize_windows_component)
            .filter(|value| !value.is_empty())
            .unwrap_or_default(),
        Placeholder::Date => capture_date(context.captured_at),
        Placeholder::Time => capture_time(context.captured_at),
        Placeholder::DateTime => {
            let date = capture_date(context.captured_at);
            let time = capture_time(context.captured_at);
            if date.is_empty() || time.is_empty() {
                String::new()
            } else {
                format!("{date}-{time}")
            }
        }
        Placeholder::Id => short_identifier(context.capture_item_id),
        Placeholder::Classification => sanitize_windows_component(context.classification),
        Placeholder::Seq => seq.to_string(),
    }
}

/// Collapses runs of the configured separator that surround empty placeholder
/// values, so `{source} - {character} - {id}` with no character renders as
/// `shot - id` instead of `shot -  - id`. This keeps the default template
/// byte-compatible with the legacy archive naming for scene/private captures.
fn collapse_repeated_separators(rendered: &str, separator: &str) -> String {
    if separator.is_empty() {
        return rendered.to_owned();
    }
    let repeated = format!("{separator}{separator}");
    let mut collapsed = rendered.to_owned();
    while collapsed.contains(&repeated) {
        collapsed = collapsed.replace(&repeated, separator);
    }
    collapsed
}

/// Extracts `yyyyMMdd` from a UTC RFC 3339 string such as
/// `2026-08-06T09:17:26.653Z`; empty when the string is malformed.
fn capture_date(captured_at: Option<&str>) -> String {
    let value = captured_at.unwrap_or_default();
    let bytes = value.as_bytes();
    if bytes.len() >= 10
        && bytes
            .iter()
            .take(10)
            .all(|byte| byte.is_ascii_digit() || *byte == b'-')
    {
        value[..10]
            .chars()
            .filter(|character| character.is_ascii_digit())
            .collect()
    } else {
        String::new()
    }
}

/// Extracts `HHmmss` from a UTC RFC 3339 string; empty when malformed.
fn capture_time(captured_at: Option<&str>) -> String {
    let value = captured_at.unwrap_or_default();
    if value.len() >= 19 && value.as_bytes()[10] == b'T' {
        value[11..19]
            .chars()
            .filter(|character| character.is_ascii_digit())
            .collect()
    } else {
        String::new()
    }
}

/// Windows-safe component cleaning shared by archive naming and the archive
/// service: replaces characters that Windows forbids in file names, applies
/// the 80-character component limit, trims trailing dots/spaces and prefixes
/// reserved device names (CON, PRN, AUX, NUL, COM1..9, LPT1..9).
pub fn sanitize_windows_component(value: &str) -> String {
    let mut sanitized: String = value
        .chars()
        .map(|character| {
            if character.is_control()
                || matches!(
                    character,
                    '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
                )
            {
                '_'
            } else {
                character
            }
        })
        .take(80)
        .collect();
    sanitized = sanitized.trim().trim_end_matches(['.', ' ']).to_owned();
    if sanitized.is_empty() {
        return "capture".to_owned();
    }

    let upper = sanitized.to_ascii_uppercase();
    let reserved = matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (upper.len() == 4
            && (upper.starts_with("COM") || upper.starts_with("LPT"))
            && upper.as_bytes()[3].is_ascii_digit()
            && upper.as_bytes()[3] != b'0');
    if reserved {
        sanitized.insert(0, '_');
    }
    sanitized
}

/// First eight alphanumeric characters of an id, stable and file-name safe.
pub fn short_identifier(value: &str) -> String {
    let short: String = value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .take(8)
        .collect();
    if short.is_empty() {
        "capture".to_owned()
    } else {
        short
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::archive_naming::ArchiveNamingSettings;

    fn context<'a>(
        source: &'a str,
        character: Option<&'a str>,
        id: &'a str,
        classification: &'a str,
        captured_at: Option<&'a str>,
        avatar: bool,
    ) -> ArchiveNameContext<'a> {
        ArchiveNameContext {
            source_stem: source,
            character_name: character,
            capture_item_id: id,
            classification,
            captured_at,
            avatar,
        }
    }

    #[test]
    fn default_template_matches_legacy_names() {
        let settings = ArchiveNamingSettings::default();
        let ctx = context(
            "shot-001",
            Some("Ava:Star"),
            "a1b2c3d4e5f6",
            "person",
            Some("2026-08-06T09:17:26.653Z"),
            false,
        );
        assert_eq!(render(&settings, &ctx), "shot-001 - Ava_Star - a1b2c3d4");
        let avatar_ctx = context(
            "shot-001",
            Some("Ava"),
            "a1b2c3d4e5f6",
            "person",
            Some("2026-08-06T09:17:26.653Z"),
            true,
        );
        assert_eq!(
            render(&settings, &avatar_ctx),
            "shot-001 - Ava - a1b2c3d4-avatar"
        );
    }

    #[test]
    fn renders_all_placeholders() {
        let settings = ArchiveNamingSettings {
            template: "{source}_{character}_{date}_{time}_{datetime}_{id}_{classification}_{seq}"
                .to_owned(),
            separator: "-".to_owned(),
        };
        let ctx = context(
            "shot",
            Some("Ava"),
            "abc_123XYZ",
            "private",
            Some("2026-08-06T09:17:26.653Z"),
            false,
        );
        assert_eq!(
            render_with_seq(&settings, &ctx, 7),
            "shot_Ava_20260806_091726_20260806-091726_abc123XY_private_7"
        );
    }

    #[test]
    fn appends_short_id_when_template_lacks_id_and_seq() {
        let settings = ArchiveNamingSettings {
            template: "{source} {character}".to_owned(),
            separator: " - ".to_owned(),
        };
        let ctx = context("shot", Some("Ava"), "a1b2c3d4", "person", None, false);
        assert_eq!(render(&settings, &ctx), "shot Ava - a1b2c3d4");
    }

    #[test]
    fn keeps_id_when_template_contains_it() {
        let settings = ArchiveNamingSettings {
            template: "{id} {source}".to_owned(),
            separator: " - ".to_owned(),
        };
        let ctx = context("shot", None, "a1b2c3d4", "person", None, false);
        assert_eq!(render(&settings, &ctx), "a1b2c3d4 shot");
    }

    #[test]
    fn collapses_separators_around_missing_character_like_legacy_names() {
        let settings = ArchiveNamingSettings::default();
        let ctx = context("shot-001", None, "a1b2c3d4e5f6", "scene", None, false);
        assert_eq!(render(&settings, &ctx), "shot-001 - a1b2c3d4");
        let avatar_ctx = context("shot-001", None, "a1b2c3d4e5f6", "person", None, true);
        assert_eq!(render(&settings, &avatar_ctx), "shot-001 - a1b2c3d4-avatar");
    }

    #[test]
    fn rejects_unknown_placeholders_and_empty_templates() {
        assert!(matches!(
            Template::parse("{source} - {unknown}"),
            Err(AppError::Validation(_))
        ));
        assert!(matches!(
            Template::parse("   "),
            Err(AppError::Validation(_))
        ));
        assert!(Template::parse("{source}{id}").is_ok());
    }

    #[test]
    fn renders_empty_character_and_malformed_timestamps_safely() {
        let settings = ArchiveNamingSettings {
            template: "{character}|{date}|{time}|{datetime}".to_owned(),
            separator: " - ".to_owned(),
        };
        let ctx = context("shot", None, "id", "person", Some("not-a-date"), false);
        assert_eq!(render(&settings, &ctx), "___ - id");
    }

    #[test]
    fn sanitizes_assembled_name_and_reserved_prefix() {
        let settings = ArchiveNamingSettings {
            template: "{source} {id}".to_owned(),
            separator: "-".to_owned(),
        };
        let ctx = context("CON", None, "a1b2c3d4", "person", None, false);
        assert_eq!(render(&settings, &ctx), "_CON a1b2c3d4");
    }
}
