//! The on-disk motion library: built-ins plus JSON files from the user folder
//! and installed extensions. Each file holds one preset or an array of them.
use crate::preset::{builtin_motions, MotionKind, MotionPreset};
use std::fs;
use std::path::{Path, PathBuf};

/// Folder name of a motion library under the repo root or an extension.
pub const MOTIONS_DIR: &str = "motions";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MotionSource {
    Builtin,
    /// Saved by the user; the file can be rewritten or removed.
    User {
        file: PathBuf,
    },
    /// Shipped by an extension (read-only).
    Extension {
        id: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct LibraryEntry {
    pub preset: MotionPreset,
    pub source: MotionSource,
}

/// All presets available to the editor, in precedence order: built-ins, then
/// the user library, then extensions. A later duplicate id is skipped with a
/// warning so a preset id always means one motion.
#[derive(Debug, Clone, Default)]
pub struct MotionLibrary {
    entries: Vec<LibraryEntry>,
    user_dir: Option<PathBuf>,
}

impl MotionLibrary {
    /// Built-ins only (tests, or when no library folder exists).
    pub fn builtin() -> Self {
        Self {
            entries: builtin_motions()
                .iter()
                .cloned()
                .map(|preset| LibraryEntry {
                    preset,
                    source: MotionSource::Builtin,
                })
                .collect(),
            user_dir: None,
        }
    }

    /// Load `user_dir` and each `(extension id, extension dir)`'s `motions/`.
    /// Unreadable or invalid files are reported, never fatal.
    pub fn load(user_dir: &Path, extensions: &[(String, PathBuf)]) -> (Self, Vec<String>) {
        let mut library = Self::builtin();
        library.user_dir = Some(user_dir.to_path_buf());
        let mut warnings = Vec::new();
        for (file, preset) in read_dir(user_dir, &mut warnings) {
            library.insert(preset, MotionSource::User { file }, &mut warnings);
        }
        for (id, dir) in extensions {
            for (_, preset) in read_dir(&dir.join(MOTIONS_DIR), &mut warnings) {
                library.insert(
                    preset,
                    MotionSource::Extension { id: id.clone() },
                    &mut warnings,
                );
            }
        }
        (library, warnings)
    }

    fn insert(&mut self, preset: MotionPreset, source: MotionSource, warnings: &mut Vec<String>) {
        if self.get(&preset.id).is_some() {
            warnings.push(format!("motion `{}` already exists; skipped", preset.id));
            return;
        }
        self.entries.push(LibraryEntry { preset, source });
    }

    pub fn entries(&self) -> &[LibraryEntry] {
        &self.entries
    }

    pub fn presets(&self) -> impl Iterator<Item = &MotionPreset> {
        self.entries.iter().map(|entry| &entry.preset)
    }

    pub fn of_kind(&self, kind: MotionKind) -> impl Iterator<Item = &MotionPreset> {
        self.presets().filter(move |preset| preset.kind == kind)
    }

    pub fn get(&self, id: &str) -> Option<&MotionPreset> {
        self.presets().find(|preset| preset.id == id)
    }

    pub fn entry(&self, id: &str) -> Option<&LibraryEntry> {
        self.entries.iter().find(|entry| entry.preset.id == id)
    }

    /// Save (or overwrite) a user preset as `<user_dir>/<id>.json`.
    pub fn save_user(&mut self, preset: MotionPreset) -> Result<PathBuf, String> {
        preset.validate()?;
        let dir = self
            .user_dir
            .clone()
            .ok_or("the motion library has no user folder")?;
        match self.entry(&preset.id).map(|entry| &entry.source) {
            None | Some(MotionSource::User { .. }) => {}
            Some(_) => return Err(format!("motion `{}` is built in or read-only", preset.id)),
        }
        fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
        let file = dir.join(format!("{}.json", preset.id));
        let text = serde_json::to_string_pretty(&preset).map_err(|error| error.to_string())?;
        // Write-then-rename so a crash never leaves a half-written preset.
        let partial = file.with_extension("json.partial");
        fs::write(&partial, text).map_err(|error| error.to_string())?;
        fs::rename(&partial, &file).map_err(|error| error.to_string())?;
        self.entries.retain(|entry| entry.preset.id != preset.id);
        self.entries.push(LibraryEntry {
            preset,
            source: MotionSource::User { file: file.clone() },
        });
        Ok(file)
    }

    /// Delete a user preset. Built-in and extension presets stay.
    pub fn remove_user(&mut self, id: &str) -> Result<(), String> {
        let Some(LibraryEntry {
            source: MotionSource::User { file },
            ..
        }) = self.entry(id).cloned()
        else {
            return Err(format!("motion `{id}` is not a user preset"));
        };
        // A file may hold several presets; rewrite it without this one.
        let rest: Vec<MotionPreset> = parse_file(&file)?
            .into_iter()
            .filter(|preset| preset.id != id)
            .collect();
        if rest.is_empty() {
            fs::remove_file(&file).map_err(|error| error.to_string())?;
        } else {
            let text = serde_json::to_string_pretty(&rest).map_err(|error| error.to_string())?;
            fs::write(&file, text).map_err(|error| error.to_string())?;
        }
        self.entries.retain(|entry| entry.preset.id != id);
        Ok(())
    }
}

fn parse_file(file: &Path) -> Result<Vec<MotionPreset>, String> {
    let text = fs::read_to_string(file).map_err(|error| error.to_string())?;
    let value: serde_json::Value =
        serde_json::from_str(&text).map_err(|error| error.to_string())?;
    let presets = if value.is_array() {
        serde_json::from_value(value)
    } else {
        serde_json::from_value(value).map(|preset| vec![preset])
    };
    presets.map_err(|error| error.to_string())
}

fn read_dir(dir: &Path, warnings: &mut Vec<String>) -> Vec<(PathBuf, MotionPreset)> {
    let Ok(read) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = read
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    files.sort();
    let mut presets = Vec::new();
    for file in files {
        match parse_file(&file) {
            Ok(list) => {
                for preset in list {
                    match preset.validate() {
                        Ok(()) => presets.push((file.clone(), preset)),
                        Err(error) => warnings.push(format!("{}: {error}", file.display())),
                    }
                }
            }
            Err(error) => warnings.push(format!("{}: {error}", file.display())),
        }
    }
    presets
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preset::{MotionChannels, MotionCurve, MotionKey};

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("todo-motion-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn wobble() -> MotionPreset {
        MotionPreset {
            id: "wobble".into(),
            name: "摇摆".into(),
            kind: MotionKind::Intro,
            duration: 0.5,
            description: String::new(),
            tags: vec![],
            channels: MotionChannels {
                x: Some(MotionCurve(vec![
                    MotionKey {
                        t: 0.0,
                        v: 0.02,
                        ease: Default::default(),
                    },
                    MotionKey {
                        t: 1.0,
                        v: 0.0,
                        ease: Default::default(),
                    },
                ])),
                ..Default::default()
            },
            outgoing: None,
            outgoing_on_top: false,
        }
    }

    #[test]
    fn user_presets_round_trip_and_cannot_shadow_builtins() {
        let root = temp_dir();
        let user = root.join(MOTIONS_DIR);
        let (mut library, warnings) = MotionLibrary::load(&user, &[]);
        assert!(warnings.is_empty());
        let file = library.save_user(wobble()).unwrap();
        assert!(file.is_file());
        let (reloaded, _) = MotionLibrary::load(&user, &[]);
        assert_eq!(reloaded.get("wobble"), Some(&wobble()));

        let mut fake = wobble();
        fake.id = "pop".into();
        assert!(library.save_user(fake).is_err());

        library.remove_user("wobble").unwrap();
        assert!(!file.exists());
        assert!(library.remove_user("pop").is_err());
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn extension_presets_load_and_bad_files_warn() {
        let root = temp_dir();
        let ext = root.join("ext");
        fs::create_dir_all(ext.join(MOTIONS_DIR)).unwrap();
        fs::write(
            ext.join(MOTIONS_DIR).join("set.json"),
            serde_json::to_string(&vec![wobble()]).unwrap(),
        )
        .unwrap();
        fs::write(ext.join(MOTIONS_DIR).join("broken.json"), "{").unwrap();
        let (library, warnings) =
            MotionLibrary::load(&root.join("none"), &[("demo.ext".into(), ext)]);
        assert_eq!(
            library.entry("wobble").unwrap().source,
            MotionSource::Extension {
                id: "demo.ext".into()
            }
        );
        assert_eq!(warnings.len(), 1);
        fs::remove_dir_all(root).ok();
    }
}

#[cfg(test)]
mod shipped {
    /// Every motion library shipped with a built-in extension is valid.
    #[test]
    fn builtin_extension_motions_load_without_warnings() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let dir = root.join("extensions/builtin/spongebob-koubo");
        let (library, warnings) = super::MotionLibrary::load(
            &root.join("target/no-user-motions"),
            &[("studio.spongebob-koubo".into(), dir)],
        );
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(library.get("sb.cardRise").is_some());
    }
}
