//! The server's font source: what the browser gets from the Local Font Access API, the backend
//! gets from the system font database.
//!
//! Two jobs use it. **Outlining** a label (MCP `outline_text`) needs the raw face bytes so the core
//! can shape glyphs; **rendering** a preview (`render_document`) needs the same faces so a `<text>`
//! shows up as words rather than nothing. Loading system fonts costs a beat, so it happens once per
//! process and both jobs share the result.
//!
//! The deployed image bundles DejaVu + Liberation (see the Dockerfile) precisely so this isn't
//! empty on a `scratch` runtime; `NIB_FONT_DIR` adds a mounted directory on top, for brand faces a
//! self-hosted instance wants. With no faces at all this degrades to "no face found" — outlining
//! reports that plainly instead of producing empty geometry, and previews render text-free.
//!
//! There is no fontconfig here, so the two things fontconfig would normally do are done explicitly:
//! resolving the generic families, and aliasing the names documents actually ask for onto the faces
//! that exist.

use std::sync::{Arc, OnceLock};

use nib_core::model::tree::TextInfo;

static DB: OnceLock<Arc<fontdb::Database>> = OnceLock::new();

/// Faces that stand in for a family the host doesn't have, best substitute first. Liberation is
/// **metric-compatible** with the Microsoft core fonts, so a label authored in Arial outlines with
/// the same advances — its width, centring and line breaks survive the substitution, which a
/// merely similar-looking face wouldn't guarantee.
const ALIASES: &[(&str, &[&str])] = &[
    ("arial", &["Liberation Sans", "DejaVu Sans"]),
    ("helvetica", &["Liberation Sans", "DejaVu Sans"]),
    ("helvetica neue", &["Liberation Sans", "DejaVu Sans"]),
    ("segoe ui", &["Liberation Sans", "DejaVu Sans"]),
    ("roboto", &["Liberation Sans", "DejaVu Sans"]),
    ("inter", &["Liberation Sans", "DejaVu Sans"]),
    ("verdana", &["DejaVu Sans", "Liberation Sans"]),
    ("tahoma", &["DejaVu Sans", "Liberation Sans"]),
    ("times", &["Liberation Serif", "DejaVu Serif"]),
    ("times new roman", &["Liberation Serif", "DejaVu Serif"]),
    ("georgia", &["Liberation Serif", "DejaVu Serif"]),
    ("garamond", &["Liberation Serif", "DejaVu Serif"]),
    ("courier", &["Liberation Mono", "DejaVu Sans Mono"]),
    ("courier new", &["Liberation Mono", "DejaVu Sans Mono"]),
    ("menlo", &["DejaVu Sans Mono", "Liberation Mono"]),
    ("monaco", &["DejaVu Sans Mono", "Liberation Mono"]),
    ("consolas", &["Liberation Mono", "DejaVu Sans Mono"]),
];

/// Preferences for each generic family, best first — what `font-family="sans-serif"` resolves to.
const GENERIC_SANS: &[&str] = &["Liberation Sans", "DejaVu Sans", "Helvetica", "Arial"];
const GENERIC_SERIF: &[&str] = &[
    "Liberation Serif",
    "DejaVu Serif",
    "Times New Roman",
    "Georgia",
];
const GENERIC_MONO: &[&str] = &[
    "Liberation Mono",
    "DejaVu Sans Mono",
    "Menlo",
    "Courier New",
];

/// The alias chain for `name` — the name itself first, then any substitutes for it.
fn with_aliases(name: &str) -> Vec<String> {
    let mut out = vec![name.to_string()];
    if let Some((_, subs)) = ALIASES
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name.trim()))
    {
        out.extend(subs.iter().map(|s| s.to_string()));
    }
    out
}

/// Is a face by this family name loaded?
fn has_family(db: &fontdb::Database, name: &str) -> bool {
    db.faces().any(|f| {
        f.families
            .iter()
            .any(|(fam, _)| fam.eq_ignore_ascii_case(name))
    })
}

/// Point the generic families at faces that actually exist. fontdb defaults them to the Microsoft
/// core fonts, which a Linux image doesn't have — leaving `sans-serif` unresolvable and every such
/// label unrendered.
fn resolve_generics(db: &mut fontdb::Database) {
    let pick = |db: &fontdb::Database, names: &[&str]| {
        names
            .iter()
            .find(|n| has_family(db, n))
            .map(|n| n.to_string())
    };
    if let Some(n) = pick(db, GENERIC_SANS) {
        db.set_sans_serif_family(n);
    }
    if let Some(n) = pick(db, GENERIC_SERIF) {
        db.set_serif_family(n);
    }
    if let Some(n) = pick(db, GENERIC_MONO) {
        db.set_monospace_family(n);
    }
}

/// The process-wide font database: the host's own fonts (the image bundles a set), plus anything
/// in `NIB_FONT_DIR`. Loaded on first use — scanning takes a beat, and both callers share it.
pub fn database() -> Arc<fontdb::Database> {
    DB.get_or_init(|| {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        if let Ok(dir) = std::env::var("NIB_FONT_DIR")
            && !dir.trim().is_empty()
        {
            db.load_fonts_dir(dir.trim());
        }
        resolve_generics(&mut db);
        tracing::info!(faces = db.len(), "font database loaded");
        Arc::new(db)
    })
    .clone()
}

/// The CSS weight keyword/number a label asks for, as fontdb understands it.
fn weight_of(info: &TextInfo) -> fontdb::Weight {
    match info.weight.trim().to_lowercase().as_str() {
        "normal" => fontdb::Weight::NORMAL,
        "bold" => fontdb::Weight::BOLD,
        "lighter" => fontdb::Weight::LIGHT,
        "bolder" => fontdb::Weight::EXTRA_BOLD,
        other => other
            .parse::<u16>()
            .map(fontdb::Weight)
            .unwrap_or(fontdb::Weight::NORMAL),
    }
}

fn style_of(info: &TextInfo) -> fontdb::Style {
    match info.style.trim().to_lowercase().as_str() {
        "italic" => fontdb::Style::Italic,
        "oblique" => fontdb::Style::Oblique,
        _ => fontdb::Style::Normal,
    }
}

/// Split a CSS `font-family` list into bare names, quotes removed.
fn families(list: &str) -> Vec<String> {
    list.split(',')
        .map(|f| f.trim().trim_matches(['"', '\'']).to_string())
        .filter(|f| !f.is_empty())
        .collect()
}

/// The face bytes to shape `info` with, plus which face of a collection they are — the two
/// arguments `Editor::text_outline_d` needs. `None` when the host has no font that answers the
/// family list (including no fonts at all).
pub fn face_for(info: &TextInfo) -> Option<(Vec<u8>, u32)> {
    let db = database();
    // Each authored name, then its substitutes — an alias chain rather than a single name, since
    // the host is unlikely to have the exact face a designer used.
    let names: Vec<String> = families(&info.family)
        .iter()
        .flat_map(|name| with_aliases(name))
        .collect();
    let mut requested: Vec<fontdb::Family> = names
        .iter()
        .map(|name| match name.to_lowercase().as_str() {
            "sans-serif" | "system-ui" | "ui-sans-serif" => fontdb::Family::SansSerif,
            "serif" | "ui-serif" => fontdb::Family::Serif,
            "monospace" | "ui-monospace" => fontdb::Family::Monospace,
            "cursive" => fontdb::Family::Cursive,
            "fantasy" => fontdb::Family::Fantasy,
            _ => fontdb::Family::Name(name),
        })
        .collect();
    // An unnamed or unmatched family still deserves an answer — whatever the host calls sans-serif.
    requested.push(fontdb::Family::SansSerif);

    let id = db.query(&fontdb::Query {
        families: &requested,
        weight: weight_of(info),
        stretch: fontdb::Stretch::Normal,
        style: style_of(info),
    })?;
    db.with_face_data(id, |data, index| (data.to_vec(), index))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(family: &str) -> TextInfo {
        TextInfo {
            uid: "u".into(),
            name: String::new(),
            text: "Hi".into(),
            family: family.into(),
            weight: "normal".into(),
            style: "normal".into(),
            font_size: 16.0,
            x: 0.0,
            y: 0.0,
            letter_spacing: 0.0,
            anchor: "start".into(),
        }
    }

    /// There is no fontconfig in a scratch image, so substitution is ours to do: a document
    /// authored in Arial has to reach the metric-compatible Liberation Sans the image bundles,
    /// with the authored name still tried first for a host that does have it.
    #[test]
    fn aliases_reach_the_faces_a_linux_image_actually_ships() {
        assert_eq!(
            with_aliases("Arial"),
            vec!["Arial", "Liberation Sans", "DejaVu Sans"]
        );
        // Case- and space-insensitive on the authored name.
        assert_eq!(with_aliases("times new roman")[1], "Liberation Serif");
        assert_eq!(with_aliases(" Consolas ")[1], "Liberation Mono");
        // A face with no substitutes is simply itself — the query then falls back to sans-serif.
        assert_eq!(with_aliases("Futura"), vec!["Futura"]);
    }

    /// fontdb defaults the generics to Microsoft core fonts, which no Linux image has; without
    /// this every `font-family="sans-serif"` label would be unresolvable.
    #[test]
    fn generic_families_resolve_to_faces_that_exist() {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        if db.is_empty() {
            return; // a host with no fonts — the condition the tools report to the caller
        }
        resolve_generics(&mut db);
        assert!(
            has_family(&db, db.family_name(&fontdb::Family::SansSerif)),
            "sans-serif names a loaded face"
        );
    }

    /// The whole point of bundling: a label asking for a font the host lacks still resolves to
    /// *something* shapeable rather than failing the conversion.
    #[test]
    fn an_unknown_family_still_resolves_to_a_face() {
        let db = database();
        if db.is_empty() {
            return;
        }
        let (bytes, _) = face_for(&info("Nonexistent Brand Face, sans-serif"))
            .expect("falls back to a real face");
        assert!(bytes.len() > 1000, "real font bytes");
    }
}
