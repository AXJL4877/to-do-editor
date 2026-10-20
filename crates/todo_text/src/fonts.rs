use crate::TextError;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use ttf_parser::{Face, GlyphId};

/// Font files resolved by plain file name against ordered directories.
/// Loaded bytes are kept for the library's lifetime.
pub struct FontLibrary {
    dirs: Vec<PathBuf>,
    loaded: HashMap<String, Option<Arc<[u8]>>>,
}

impl FontLibrary {
    pub fn new(dirs: impl IntoIterator<Item = PathBuf>) -> Self {
        Self {
            dirs: dirs.into_iter().collect(),
            loaded: HashMap::new(),
        }
    }

    pub fn dirs(&self) -> &[PathBuf] {
        &self.dirs
    }

    /// First directory containing `file`. Names with path components are refused,
    /// so style documents cannot reach outside the font directories.
    pub fn locate(&self, file: &str) -> Option<PathBuf> {
        let name = Path::new(file);
        let plain = !file.is_empty()
            && name.components().count() == 1
            && name.file_name().is_some_and(|n| n == file);
        if !plain {
            return None;
        }
        self.dirs
            .iter()
            .map(|dir| dir.join(file))
            .find(|path| path.is_file())
    }

    fn load(&mut self, file: &str) -> Option<Arc<[u8]>> {
        if let Some(cached) = self.loaded.get(file) {
            return cached.clone();
        }
        let bytes = self
            .locate(file)
            .and_then(|path| std::fs::read(path).ok())
            .map(Arc::<[u8]>::from);
        self.loaded.insert(file.to_owned(), bytes.clone());
        bytes
    }

    /// Primary font followed by available fallbacks. Missing fallbacks are skipped;
    /// if the primary is missing the first available fallback takes its place.
    pub(crate) fn chain(
        &mut self,
        primary: &str,
        fallbacks: &[String],
    ) -> Result<FontChain, TextError> {
        let mut data = Vec::new();
        for file in std::iter::once(primary).chain(fallbacks.iter().map(String::as_str)) {
            if let Some(bytes) = self.load(file) {
                if Face::parse(&bytes, 0).is_err() {
                    return Err(TextError::InvalidFont(file.to_owned()));
                }
                data.push(bytes);
            }
        }
        if data.is_empty() {
            return Err(TextError::FontNotFound(primary.to_owned()));
        }
        Ok(FontChain { data })
    }
}

/// Ordered font data; faces are parsed on demand because they borrow the bytes.
pub(crate) struct FontChain {
    data: Vec<Arc<[u8]>>,
}

impl FontChain {
    pub fn faces(&self) -> Vec<Face<'_>> {
        self.data
            .iter()
            .map(|bytes| Face::parse(bytes, 0).expect("validated when loaded"))
            .collect()
    }
}

/// First face with a real glyph for `ch`; `.notdef` of the primary otherwise.
pub(crate) fn lookup(faces: &[Face<'_>], ch: char) -> (usize, GlyphId) {
    faces
        .iter()
        .enumerate()
        .find_map(|(index, face)| {
            face.glyph_index(ch)
                .filter(|id| id.0 != 0)
                .map(|id| (index, id))
        })
        .unwrap_or((0, GlyphId(0)))
}

/// The family name a font file declares: the Simplified Chinese name when it
/// has one, else the first readable one.
pub fn family_name(bytes: &[u8]) -> Option<String> {
    let face = Face::parse(bytes, 0).ok()?;
    let mut names: Vec<(bool, String)> = face
        .names()
        .into_iter()
        .filter(|name| {
            name.name_id == ttf_parser::name_id::TYPOGRAPHIC_FAMILY
                || name.name_id == ttf_parser::name_id::FAMILY
        })
        .filter_map(|name| {
            let chinese = name.is_unicode() && name.language_id == 0x0804;
            Some((chinese, name.to_string()?))
        })
        .filter(|(_, text)| !text.trim().is_empty())
        .collect();
    names.sort_by_key(|(chinese, _)| !chinese);
    names.into_iter().next().map(|(_, text)| text.trim().to_owned())
}
