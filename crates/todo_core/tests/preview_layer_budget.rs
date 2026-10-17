//! Preview stacks: stills and text are cheap, only decoded video has its own budget.
use todo_core::*;

fn track(id: String, kind: TrackKind) -> Track {
    Track {
        name: id.clone(),
        id,
        kind,
        muted: false,
        locked: false,
        clips: vec![],
    }
}

fn media(id: &str, kind: MediaKind) -> MediaItem {
    let name = match kind {
        MediaKind::Image => format!("{id}.png"),
        _ => format!("{id}.mp4"),
    };
    MediaItem {
        id: id.into(),
        path: Some(name.clone()),
        name,
        duration: 10.0,
        has_audio: Some(false),
        width: Some(64),
        height: Some(64),
        kind: Some(kind),
        folder_id: None,
        alpha: None,
        reveal: None,
    }
}

/// `kinds[0]` is the topmost visual layer (first track).
fn stack(kinds: &[MediaKind]) -> Project {
    let mut project = create_project(None, None, None, None, None);
    project.tracks = (0..kinds.len())
        .map(|index| track(format!("v{index}"), TrackKind::Video))
        .collect();
    for (index, kind) in kinds.iter().enumerate() {
        let id = format!("m{index}");
        project.media.push(media(&id, *kind));
        project = add_clip(
            project,
            &format!("v{index}"),
            normalize_clip(format!("c{index}"), id, 0.0, 4.0, 0.0, 1.0),
        );
    }
    project
}

fn ids(project: &Project) -> Vec<String> {
    resolve_layers(project, 1.0)
        .into_iter()
        .map(|hit| hit.clip.id)
        .collect()
}

#[test]
fn still_layers_beyond_the_decoder_budget_stay_visible() {
    let project = stack(&[MediaKind::Image; 7]);
    assert_eq!(ids(&project).len(), 7);
}

#[test]
fn decoded_video_keeps_the_topmost_layers() {
    let mut kinds = vec![MediaKind::Video; MAX_DECODED_LAYERS + 2];
    kinds.push(MediaKind::Image);
    let layers = ids(&stack(&kinds));
    // The two deepest videos are dropped; the still below them is kept.
    assert_eq!(layers.len(), MAX_DECODED_LAYERS + 1);
    assert_eq!(layers.first().unwrap(), &format!("c{}", kinds.len() - 1));
    assert_eq!(layers.last().unwrap(), "c0");
    for dropped in [MAX_DECODED_LAYERS, MAX_DECODED_LAYERS + 1] {
        assert!(!layers.contains(&format!("c{dropped}")));
    }
}

#[test]
fn total_stack_keeps_the_topmost_layers() {
    let project = stack(&[MediaKind::Image; MAX_PREVIEW_LAYERS + 3]);
    let layers = ids(&project);
    assert_eq!(layers.len(), MAX_PREVIEW_LAYERS);
    assert_eq!(layers.last().unwrap(), "c0");
    assert_eq!(
        layers.first().unwrap(),
        &format!("c{}", MAX_PREVIEW_LAYERS - 1)
    );
}

#[test]
fn budgets_fit_inside_the_total() {
    const { assert!(MAX_DECODED_LAYERS + MAX_TEXT_LAYERS <= MAX_PREVIEW_LAYERS) };
}
