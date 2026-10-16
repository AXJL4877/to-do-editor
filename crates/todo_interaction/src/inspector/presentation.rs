use todo_core::{
    builtin_text_styles, clip_display_name, find_clip, is_visual_media, ClipSource, ClipTransform,
    Project, TextContent,
};

#[derive(Debug, Clone, PartialEq)]
pub struct ClipProperties {
    pub clip_id: String,
    pub name: String,
    pub start: f64,
    pub duration: f64,
    pub source: SourceProperties,
    pub visual: bool,
    pub locked: bool,
    pub transform: ClipTransform,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SourceProperties {
    Media { offset: f64, rate: f64 },
    Text(TextProperties),
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextProperties {
    pub content: TextContent,
    /// Project styles first, then built-ins not yet copied into the project.
    pub styles: Vec<StyleOption>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyleOption {
    pub id: String,
    pub name: String,
}

pub(crate) fn project_properties(project: &Project, id: Option<&str>) -> Option<ClipProperties> {
    let found = find_clip(project, id?)?;
    let clip = &found.clip;
    let (source, media) = match &clip.source {
        ClipSource::Media(source) => {
            let media = project.media.iter().find(|m| m.id == source.media_id);
            (
                SourceProperties::Media {
                    offset: source.offset,
                    rate: source.rate,
                },
                media,
            )
        }
        ClipSource::Text(content) => (
            SourceProperties::Text(TextProperties {
                content: content.clone(),
                styles: style_options(project),
            }),
            None,
        ),
    };
    Some(ClipProperties {
        clip_id: clip.id.clone(),
        name: clip_display_name(clip, media),
        start: clip.start,
        duration: clip.duration,
        visual: clip.is_text() || media.is_some_and(is_visual_media),
        source,
        locked: found.track.locked,
        transform: clip.transform,
    })
}

fn style_options(project: &Project) -> Vec<StyleOption> {
    let mut options: Vec<StyleOption> = project
        .text_styles
        .iter()
        .map(|style| StyleOption {
            id: style.id.clone(),
            name: style.name.clone(),
        })
        .collect();
    for style in builtin_text_styles() {
        if !options.iter().any(|option| option.id == style.id) {
            options.push(StyleOption {
                id: style.id,
                name: style.name,
            });
        }
    }
    options
}
