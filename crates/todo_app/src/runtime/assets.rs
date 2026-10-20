//! Left-column motion / font libraries: load from disk and answer panel actions.
use super::App;
use crate::adapters::{motions, paths};
use std::path::PathBuf;
use todo_interaction::{FontInput, FONT_SAMPLE, STYLE_SAMPLE};
use todo_media::{font_catalog, font_preview, packaged_font_dirs, style_preview};
use todo_motion::{MotionLibrary, MOTIONS_DIR};
use todo_text::FontLibrary;
use todo_ui::{AssetsAction, LeftTab, ThumbCache};

impl App {
    pub(super) fn load_asset_libraries(repo: &std::path::Path) -> AssetLibraries {
        let (motion_library, mut warnings) = motions::load(repo);
        let motions_dir = repo.join(MOTIONS_DIR);
        let _ = std::fs::create_dir_all(&motions_dir);
        let fonts_dir = packaged_font_dirs(repo)
            .into_iter()
            .next()
            .unwrap_or_else(|| repo.join("dependencies").join("fonts"));
        let (files, font_warnings) = font_catalog(repo);
        warnings.extend(font_warnings);
        let fonts = files
            .into_iter()
            .map(|font| FontInput {
                file: font.file,
                family: font.family,
                name: font.record.as_ref().map(|r| r.name.clone()),
                source: font
                    .record
                    .as_ref()
                    .map(|r| r.source.clone())
                    .unwrap_or_default(),
                license: font
                    .record
                    .as_ref()
                    .map(|r| r.license.clone())
                    .unwrap_or_default(),
                note: font
                    .record
                    .as_ref()
                    .map(|r| r.note.clone())
                    .unwrap_or_default(),
            })
            .collect();
        AssetLibraries {
            motion_library,
            fonts,
            warnings,
            motions_dir: motions_dir.to_string_lossy().replace('\\', "/"),
            fonts_dir: fonts_dir.to_string_lossy().replace('\\', "/"),
        }
    }

    pub(super) fn apply_asset_libraries(&mut self, loaded: AssetLibraries) {
        self.workspace
            .set_motion_library(loaded.motion_library.presets().cloned().collect());
        self.motion_library = loaded.motion_library;
        self.fonts = loaded.fonts;
        self.asset_warnings = loaded.warnings;
        self.motions_dir = loaded.motions_dir;
        self.fonts_dir = loaded.fonts_dir;
    }

    pub(super) fn reload_asset_libraries(&mut self) {
        let loaded = Self::load_asset_libraries(&paths::repo_root());
        self.apply_asset_libraries(loaded);
        self.notice("已刷新动效库和字体库".into());
    }

    pub(super) fn handle_assets_action(&mut self, action: AssetsAction, tab: LeftTab) {
        match action {
            AssetsAction::None => {}
            AssetsAction::Refresh => {
                self.reload_asset_libraries();
                if tab == LeftTab::Fonts {
                    let root = paths::repo_root();
                    fill_font_previews(&root, &self.fonts, None, &mut self.thumbs);
                }
            }
            AssetsAction::OpenFolder(path) => open_folder(&path),
            AssetsAction::SaveAsMine(id) => self.save_motion_as_mine(&id),
        }
    }

    fn save_motion_as_mine(&mut self, id: &str) {
        let Some(entry) = self.motion_library.entry(id).cloned() else {
            self.notice(format!("动效 `{id}` 不在库里"));
            return;
        };
        let mut preset = entry.preset;
        let base = preset.id.clone();
        let mut next = format!("{base}-mine");
        let mut n = 2u32;
        while self
            .motion_library
            .entry(&next)
            .is_some_and(|e| !matches!(e.source, todo_motion::MotionSource::User { .. }))
        {
            next = format!("{base}-mine-{n}");
            n += 1;
        }
        preset.id = next.clone();
        if !preset.name.contains('（') {
            preset.name = format!("{}（我的）", preset.name);
        }
        match self.motion_library.save_user(preset) {
            Ok(path) => {
                self.workspace
                    .set_motion_library(self.motion_library.presets().cloned().collect());
                self.notice(format!(
                    "已复制为 `{next}` → {}",
                    path.display()
                ));
            }
            Err(error) => self.notice(format!("复制动效失败：{error}")),
        }
    }

}

/// Render missing font / style preview thumbnails into the cache.
pub(super) fn fill_font_previews(
    root: &std::path::Path,
    catalog: &[FontInput],
    project: Option<&todo_core::Project>,
    thumbs: &mut ThumbCache,
) {
    let mut fonts = FontLibrary::new(todo_media::font_dirs(root));
    for font in catalog {
        let key = todo_interaction::font_preview_key(&font.file);
        if thumbs.contains(&key) {
            continue;
        }
        match font_preview(&mut fonts, &font.file, FONT_SAMPLE) {
            Ok(bytes) => {
                if !thumbs.insert_bytes(key, &bytes) {
                    thumbs.mark_failed(todo_interaction::font_preview_key(&font.file));
                }
            }
            Err(_) => thumbs.mark_failed(key),
        }
    }
    for (style, _) in todo_interaction::panel_styles(project) {
        let key = todo_interaction::style_preview_key(&style.id);
        if thumbs.contains(&key) {
            continue;
        }
        match style_preview(&mut fonts, &style, STYLE_SAMPLE) {
            Ok(bytes) => {
                if !thumbs.insert_bytes(key, &bytes) {
                    thumbs.mark_failed(todo_interaction::style_preview_key(&style.id));
                }
            }
            Err(_) => thumbs.mark_failed(key),
        }
    }
}

pub(super) struct AssetLibraries {
    pub motion_library: MotionLibrary,
    pub fonts: Vec<FontInput>,
    pub warnings: Vec<String>,
    pub motions_dir: String,
    pub fonts_dir: String,
}

fn open_folder(path: &str) {
    if path.is_empty() {
        return;
    }
    let path = PathBuf::from(path);
    let _ = std::fs::create_dir_all(&path);
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("explorer").arg(&path).spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = std::process::Command::new("xdg-open").arg(&path).spawn();
    }
}
