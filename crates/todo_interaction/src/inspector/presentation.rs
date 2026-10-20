use todo_core::{
    adjacent_predecessor, builtin_text_styles, clip_display_name, find_clip, is_visual_media,
    motion::builtin_motions, ClipSource, ClipTransform, MotionKind, MotionPreset, MotionRef,
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
    /// Entrance, exit and incoming transition (picture and text clips).
    pub motion: Option<MotionProperties>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MotionProperties {
    pub intro: Option<MotionRef>,
    pub outro: Option<MotionRef>,
    pub transition: Option<MotionRef>,
    /// A clip ends where this one starts on its track: a transition can play.
    pub can_transition: bool,
    pub intros: Vec<MotionOption>,
    pub outros: Vec<MotionOption>,
    pub transitions: Vec<MotionOption>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MotionOption {
    pub id: String,
    pub name: String,
    /// Suggested window length for a newly picked preset.
    pub duration: f64,
}

/// Which motion of a clip an edit targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotionSlot {
    Intro,
    Outro,
    Transition,
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

pub(crate) fn project_properties(
    project: &Project,
    id: Option<&str>,
    library: &[MotionPreset],
) -> Option<ClipProperties> {
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
    let visual = clip.is_text() || media.is_some_and(is_visual_media);
    let motion = visual.then(|| {
        let options = |kind| motion_options(project, library, kind);
        MotionProperties {
            intro: clip.intro.clone(),
            outro: clip.outro.clone(),
            transition: clip.transition.clone(),
            can_transition: adjacent_predecessor(&found.track, &clip.id).is_some(),
            intros: options(MotionKind::Intro),
            outros: options(MotionKind::Outro),
            transitions: options(MotionKind::Transition),
        }
    });
    Some(ClipProperties {
        clip_id: clip.id.clone(),
        name: clip_display_name(clip, media),
        start: clip.start,
        duration: clip.duration,
        visual,
        source,
        locked: found.track.locked,
        transform: clip.transform,
        motion,
    })
}

/// Built-ins, then presets the project carries, then the rest of the library.
fn motion_options(project: &Project, library: &[MotionPreset], kind: MotionKind) -> Vec<MotionOption> {
    let mut options: Vec<MotionOption> = Vec::new();
    for preset in builtin_motions()
        .iter()
        .chain(&project.motions)
        .chain(library)
        .filter(|preset| preset.kind == kind)
    {
        if !options.iter().any(|option| option.id == preset.id) {
            options.push(MotionOption {
                id: preset.id.clone(),
                name: preset.name.clone(),
                duration: preset.duration,
            });
        }
    }
    options
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
