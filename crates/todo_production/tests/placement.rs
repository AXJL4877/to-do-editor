use todo_core::*;
use todo_production::items::{Item, ItemFade, ItemRole, ItemSource, ItemTransform};
use todo_production::placement::{plan, record, PlacementInput, PlacementPlan};
use todo_production::production::Placement;

const WORK: &str = "data/p/productions/x/work";

fn media(file: &str, kind: MediaKind, duration: f64) -> MediaItem {
    MediaItem {
        id: format!("m-{file}"),
        name: file.into(),
        path: Some(format!("{WORK}/{file}")),
        duration,
        has_audio: Some(kind == MediaKind::Audio),
        width: (kind != MediaKind::Audio).then_some(1024),
        height: (kind != MediaKind::Audio).then_some(1024),
        kind: Some(kind),
        alpha: None,
        reveal: None,
        folder_id: None,
    }
}

fn project() -> Project {
    let mut p = create_project(None, None, None, None, None);
    for file in ["plate.png", "a.png", "b.png", "a2.png"] {
        p.media.push(media(file, MediaKind::Image, 5.0));
    }
    p.media.push(media("voice.wav", MediaKind::Audio, 6.0));
    p
}

fn item(key: &str, rev: &str, role: ItemRole, lane: u32, start: f64, file: &str) -> Item {
    Item {
        key: key.into(),
        rev: rev.into(),
        role,
        lane,
        start,
        duration: 3.0,
        source: ItemSource::File {
            file: file.into(),
            r#in: 0.0,
        },
        transform: None,
        intro: None,
        outro: None,
        transition: None,
        link: None,
        gain: None,
        fade: None,
    }
}

fn items() -> Vec<Item> {
    let mut element = item("b1/e1", "r1", ItemRole::Visual, 1, 0.5, "a.png");
    element.link = Some("g1/voice".into());
    element.fade = Some(ItemFade {
        r#in: 0.0,
        out: 0.5,
    });
    element.transform = Some(ItemTransform {
        x: 0.1,
        y: 0.0,
        scale: 0.5,
        scale_x: 1.0,
        scale_y: 1.0,
        opacity: 1.0,
    });
    vec![
        item("plate", "p", ItemRole::Visual, 0, 0.0, "plate.png"),
        element,
        item("g1/voice", "v1", ItemRole::Voice, 0, 0.0, "voice.wav"),
    ]
}

fn round(
    project: Project,
    placement: &mut Placement,
    items: &[Item],
    complete: bool,
) -> (Project, PlacementPlan) {
    let planned = plan(&PlacementInput {
        project: &project,
        production: "x",
        placement,
        items,
        workdir: WORK,
        complete,
    });
    let project = if planned.edits.is_empty() {
        project
    } else {
        execute_edit(project, EditCommand::ApplyGenerated(planned.edits.clone())).unwrap()
    };
    record(placement, &planned);
    (project, planned)
}

fn clip<'a>(p: &'a Project, key: &str) -> Option<&'a Clip> {
    p.tracks
        .iter()
        .flat_map(|t| &t.clips)
        .find(|c| c.origin.as_ref().is_some_and(|o| o.key == key))
}

fn index(p: &Project, key: &str) -> usize {
    p.tracks
        .iter()
        .position(|t| {
            t.clips
                .iter()
                .any(|c| c.origin.as_ref().is_some_and(|o| o.key == key))
        })
        .unwrap()
}

#[test]
fn places_once_and_is_idempotent() {
    let mut placement = Placement::default();
    let (p, first) = round(project(), &mut placement, &items(), false);
    assert_eq!(first.placed.len(), 3);
    assert!(
        index(&p, "b1/e1") < index(&p, "plate"),
        "elements draw above the plate"
    );
    let element = clip(&p, "b1/e1").unwrap();
    assert_eq!(
        element.outro,
        Some(todo_core::MotionRef::new("fadeOut", 0.5))
    );
    assert_eq!(element.transform.scale, 0.5);
    // Audio duration is clamped to the media.
    assert_eq!(clip(&p, "g1/voice").unwrap().duration, 3.0);
    let (_, again) = round(p, &mut placement, &items(), false);
    assert!(again.is_empty(), "{again:?}");
}

#[test]
fn a_new_revision_replaces_in_place_and_keeps_user_changes() {
    let mut placement = Placement::default();
    let (p, _) = round(project(), &mut placement, &items(), false);
    let id = clip(&p, "b1/e1").unwrap().id.clone();
    let moved = Transform {
        x: -0.3,
        ..Transform::default()
    };
    let p = execute_edit(
        p,
        EditCommand::SetTransform {
            clip_id: id.clone(),
            transform: moved,
        },
    )
    .unwrap();
    let mut next = items();
    next[1].rev = "r2".into();
    next[1].source = ItemSource::File {
        file: "a2.png".into(),
        r#in: 0.0,
    };
    let (p, _) = round(p, &mut placement, &next, false);
    let element = clip(&p, "b1/e1").unwrap();
    assert_eq!(element.id, id);
    assert_eq!(element.transform.x, -0.3, "user transform kept");
    assert_eq!(element.media_id(), Some("m-a2.png"));
    // An untouched transform follows the producer.
    next[1].rev = "r3".into();
    next[0].rev = "p2".into();
    next[0].transform = Some(ItemTransform {
        x: 0.0,
        y: 0.0,
        scale: 0.9,
        scale_x: 1.0,
        scale_y: 1.0,
        opacity: 1.0,
    });
    let (p, _) = round(p, &mut placement, &next, false);
    assert_eq!(clip(&p, "plate").unwrap().transform.scale, 0.9);
    assert_eq!(clip(&p, "b1/e1").unwrap().transform.x, -0.3);
}

#[test]
fn user_deletions_stay_deleted_until_regenerated() {
    let mut placement = Placement::default();
    let (p, _) = round(project(), &mut placement, &items(), false);
    let id = clip(&p, "b1/e1").unwrap().id.clone();
    let p = execute_edit(
        p,
        EditCommand::DeleteClips {
            clip_ids: vec![id],
            ripple: false,
        },
    )
    .unwrap();
    let (p, planned) = round(p, &mut placement, &items(), false);
    assert!(clip(&p, "b1/e1").is_none());
    assert_eq!(
        planned.tombstones,
        vec![("b1/e1".to_string(), "r1".to_string())]
    );
    let (p, again) = round(p, &mut placement, &items(), false);
    assert!(clip(&p, "b1/e1").is_none() && again.edits.is_empty());
    let mut redrawn = items();
    redrawn[1].rev = "r2".into();
    let (p, _) = round(p, &mut placement, &redrawn, false);
    assert!(clip(&p, "b1/e1").is_some());
}

#[test]
fn linked_items_follow_their_parent_and_user_moves_stick() {
    let mut placement = Placement::default();
    let (p, _) = round(project(), &mut placement, &items(), false);
    let voice = clip(&p, "g1/voice").unwrap().id.clone();
    let p = execute_edit(
        p,
        EditCommand::MoveClips {
            clip_ids: vec![voice],
            delta: 2.0,
            track_id: None,
        },
    )
    .unwrap();
    let (p, _) = round(p, &mut placement, &items(), false);
    assert_eq!(clip(&p, "g1/voice").unwrap().start, 2.0, "user move kept");
    assert_eq!(
        clip(&p, "b1/e1").unwrap().start,
        2.5,
        "linked element follows"
    );
}

#[test]
fn complete_sets_remove_what_is_no_longer_produced() {
    let mut placement = Placement::default();
    let (p, _) = round(project(), &mut placement, &items(), false);
    let fewer: Vec<Item> = items().into_iter().filter(|i| i.key != "plate").collect();
    let (p, _) = round(p.clone(), &mut placement, &fewer, false);
    assert!(clip(&p, "plate").is_some(), "partial sets never remove");
    let (p, _) = round(p, &mut placement, &fewer, true);
    assert!(clip(&p, "plate").is_none());
    assert!(!placement.placed.contains_key("plate"));
}

#[test]
fn files_without_media_are_adopted_first() {
    let mut placement = Placement::default();
    let mut list = items();
    list.push(item("b2/e1", "r1", ItemRole::Visual, 1, 4.0, "new.png"));
    let (p, planned) = round(project(), &mut placement, &list, false);
    assert_eq!(planned.adopt, vec![format!("{WORK}/new.png")]);
    assert!(clip(&p, "b2/e1").is_none() && clip(&p, "plate").is_some());
}

#[test]
fn a_clip_on_the_timeline_but_missing_from_the_record_is_adopted() {
    let mut placement = Placement::default();
    let (p, _) = round(project(), &mut placement, &items(), false);
    // The record lost the voice (an older save raced the placement) and
    // even holds a stale tombstone for it.
    let rev = placement.placed.remove("g1/voice").unwrap();
    placement
        .tombstones
        .insert(("g1/voice".into(), rev.clone()));
    let (p, planned) = round(p, &mut placement, &items(), false);
    assert!(planned.edits.is_empty(), "{planned:?}");
    assert!(clip(&p, "g1/voice").is_some());
    assert_eq!(placement.placed.get("g1/voice"), Some(&rev));
    assert!(placement.tombstones.is_empty());
    let (_, again) = round(p, &mut placement, &items(), false);
    assert!(again.is_empty(), "{again:?}");
}
