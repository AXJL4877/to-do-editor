use super::*;
use crate::control::review::Check;
use todo_core::{
    add_clip, add_track, create_project, normalize_clip, text_clip, ClipOrigin, MediaItem,
    MediaKind, TextContent, TrackKind,
};

fn project() -> Project {
    let mut p = create_project(None, None, None, None, None);
    p.media.push(MediaItem {
        id: "bg".into(),
        name: "bg.png".into(),
        path: Some("bg.png".into()),
        duration: 5.0,
        has_audio: Some(false),
        width: Some(1920),
        height: Some(1080),
        kind: Some(MediaKind::Image),
        folder_id: None,
        alpha: None,
        reveal: None,
    });
    p = add_clip(
        p,
        "v1",
        normalize_clip("a".into(), "bg".into(), 0.0, 3.0, 0.0, 1.0),
    );
    add_clip(
        p,
        "v1",
        normalize_clip("b".into(), "bg".into(), 3.0, 3.0, 0.0, 1.0),
    )
}

fn origin(key: &str) -> Option<ClipOrigin> {
    Some(ClipOrigin {
        production: "p".into(),
        key: key.into(),
        rev: "r".into(),
    })
}

/// Two produced cards, each under one spoken line, plus a title that is
/// never checked.
fn produced() -> Project {
    let mut p = project();
    p.tracks[0].clips[0].origin = origin("beat/0/card");
    p.tracks[0].clips[1].origin = origin("beat/1/card");
    p = add_track(p, TrackKind::Text);
    let text = p
        .tracks
        .iter()
        .position(|t| t.kind == TrackKind::Text)
        .unwrap();
    for (n, (start, words)) in [(0.0, "商家摆出两种杯子"), (3.0, "中杯几乎没人买")]
        .iter()
        .enumerate()
    {
        let mut clip = text_clip(
            format!("s{n}"),
            *start,
            3.0,
            TextContent::new(*words, "subtitle-kuaile"),
        );
        clip.origin = origin(&format!("sub/{n}"));
        p.tracks[text].clips.push(clip);
    }
    let mut title = text_clip(
        "t".into(),
        0.0,
        1.0,
        TextContent::new("标题", "title-red-brush"),
    );
    title.origin = origin("title/text");
    p.tracks[text].clips.push(title);
    p
}

fn checked_style() -> StyleInfo {
    let mut style = style();
    style.rubric = Some(crate::control::review::Rubric {
        checks: vec![
            Check {
                id: "line".into(),
                question: "画面和台词说的是同一件事".into(),
            },
            Check {
                id: "whole".into(),
                question: "角色完整".into(),
            },
        ],
        checked: vec!["beat/*/card".into()],
        lines: vec!["sub/".into()],
        redraw: Some("redraw".into()),
        ..Default::default()
    });
    style
}

fn frames_for(call: &Call) -> Input {
    let Call::Render { id, times } = call else {
        panic!("{call:?}")
    };
    Input::Frames {
        id: id.clone(),
        result: Ok(times.iter().map(|t| (*t, format!("f{t}.png"))).collect()),
    }
}

/// Up to the first checklist request.
fn produce_and_check(p: &Project, limits: Limits) -> (Director, Vec<Call>) {
    let (mut d, calls) = Director::with_limits(
        "t".into(),
        "做 30 秒诱饵效应".into(),
        vec![checked_style()],
        p,
        limits,
    );
    let calls = d.on(
        reply(
            id_of(&calls[0]),
            r#"{"mode":"produce","extension":"studio.spongebob-koubo","params":{"topic":"诱饵效应","redraw":"beat/0/card:9"}}"#,
        ),
        p,
    );
    assert!(matches!(&calls[0], Call::Produce { params, .. }
        if params["topic"] == "诱饵效应" && !params.contains_key("redraw")));
    let calls = d.on(Input::Produced { result: Ok(()) }, p);
    let Call::Render { times, .. } = &calls[0] else {
        panic!("{calls:?}")
    };
    assert_eq!(
        times.len(),
        2,
        "one settled frame per card, none for the title: {times:?}"
    );
    assert!(times[0] > 0.8 && times[0] < 2.55, "{times:?}");
    let calls = d.on(frames_for(&calls[0]), p);
    (d, calls)
}

#[test]
fn the_reviewer_only_answers_the_checklist_and_failed_pictures_are_redrawn() {
    let p = produced();
    let (mut d, calls) = produce_and_check(&p, Limits::default());
    assert_eq!(d.phase(), &Phase::Checking);
    let Call::Vision { id, prompt, frames } = &calls[0] else {
        panic!("{calls:?}")
    };
    assert_eq!(frames.len(), 2);
    assert!(prompt.contains("中杯几乎没人买") && prompt.contains("`whole`"));
    assert!(!prompt.contains("标题"), "the title is not checked");
    // Card 0 is unsure (passes); card 1 fails one question.
    let calls = d.on(
        reply(
            id.clone(),
            r#"{"cards":[{"index":0,"answers":{"line":"unsure","whole":"yes"}},
                         {"index":1,"answers":{"line":"yes","whole":"no","extra":"no"}}]}"#,
        ),
        &p,
    );
    assert_eq!(
        calls,
        vec![Call::Rerun {
            params: BTreeMap::from([("redraw".into(), Value::String("beat/1/card:1".into()))])
        }]
    );
    assert_eq!(d.phase(), &Phase::Redrawing { round: 1 });
    // After the rerun only the redrawn picture is looked at again.
    let calls = d.on(Input::Produced { result: Ok(()) }, &p);
    let Call::Render { times, .. } = &calls[0] else {
        panic!("{calls:?}")
    };
    assert_eq!(times.len(), 1);
    assert!(times[0] > 3.0);
    let calls = d.on(frames_for(&calls[0]), &p);
    let calls = d.on(
        reply(
            id_of(&calls[0]),
            r#"{"cards":[{"index":0,"answers":{"line":"yes","whole":"yes"}}]}"#,
        ),
        &p,
    );
    assert!(calls.is_empty());
    assert_eq!(d.phase(), &Phase::Approval);
    let approval = d.approval().unwrap();
    assert_eq!(approval.fix_rounds, 1);
    assert!(approval.remaining.is_empty(), "{approval:?}");
    assert_eq!(approval.frames.len(), 2, "the latest frame of every card");
}

#[test]
fn redraws_are_bounded_and_what_still_fails_goes_to_the_user() {
    let p = produced();
    let (mut d, mut calls) = produce_and_check(
        &p,
        Limits {
            fix_rounds: 2,
            review_frames: 1,
        },
    );
    let no = r#"{"cards":[{"index":0,"answers":{"whole":"no"}}]}"#;
    let yes = r#"{"cards":[{"index":0,"answers":{"whole":"yes"}}]}"#;
    // One frame per request: card 0 passes, card 1 fails.
    calls = d.on(reply(id_of(&calls[0]), yes), &p);
    calls = d.on(reply(id_of(&calls[0]), no), &p);
    assert!(matches!(&calls[0], Call::Rerun { params } if params["redraw"] == "beat/1/card:1"));
    for variant in [2, 0] {
        calls = d.on(Input::Produced { result: Ok(()) }, &p);
        calls = d.on(frames_for(&calls[0]), &p);
        calls = d.on(reply(id_of(&calls[0]), no), &p);
        if variant > 0 {
            assert!(matches!(&calls[0], Call::Rerun { params }
                if params["redraw"] == format!("beat/1/card:{variant}").as_str()));
        }
    }
    assert!(calls.is_empty());
    let approval = d.approval().unwrap();
    assert_eq!(approval.fix_rounds, 2);
    assert_eq!(approval.remaining.len(), 1);
    assert!(approval.remaining[0].contains("角色完整"), "{approval:?}");
}

#[test]
fn an_unreadable_answer_is_unsure_and_never_triggers_anything() {
    let p = produced();
    let (mut d, calls) = produce_and_check(&p, Limits::default());
    let calls = d.on(reply(id_of(&calls[0]), "画面整体不错，但第二张有点怪"), &p);
    assert!(calls.is_empty());
    assert_eq!(d.phase(), &Phase::Approval);
    assert!(d.approval().unwrap().remaining.is_empty());
}

#[test]
fn checked_keys_match_patterns() {
    assert!(prompts::key_matches("beat/*/card", "beat/12/card"));
    assert!(!prompts::key_matches("beat/*/card", "beat/12/text"));
    assert!(prompts::key_matches("bg/*", "bg/0"));
    assert!(!prompts::key_matches("beat/*/card", "title/card"));
}

fn style() -> StyleInfo {
    StyleInfo {
        id: "studio.spongebob-koubo".into(),
        name: "海绵宝宝口播".into(),
        description: "".into(),
        params: vec!["topic: 主题".into()],
        rubric: None,
        assets: vec![StyleAsset {
            id: "krabs-shock".into(),
            name: "蟹老板·震惊".into(),
            desc: "".into(),
            path: "D:/lib/krabs-shock.png".into(),
        }],
        draw_style: None,
    }
}

#[test]
fn editor_agent_starts_without_production_styles() {
    let p = project();
    let (director, calls) = Director::start_edit("t".into(), "给第二段加叠化".into(), &p);
    assert_eq!(director.phase(), &Phase::Editing);
    let Call::Language { messages, .. } = &calls[0] else {
        panic!("{calls:?}")
    };
    let prompt = messages
        .iter()
        .map(|message| message.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!prompt.contains("studio.spongebob-koubo"));
    assert!(!prompt.contains("海绵宝宝"));
}

fn id_of(call: &Call) -> String {
    match call {
        Call::Language { id, .. }
        | Call::Vision { id, .. }
        | Call::Render { id, .. }
        | Call::Apply { id, .. }
        | Call::Picture { id, .. } => id.clone(),
        Call::Produce { .. } | Call::Rerun { .. } => String::new(),
    }
}

fn reply(id: String, text: &str) -> Input {
    Input::Reply {
        id,
        result: Ok(text.into()),
    }
}

#[test]
fn rejected_edits_are_corrected_once_then_dropped() {
    let p = project();
    let (mut d, calls) = Director::start("t".into(), "把第二段加个叠化".into(), vec![style()], &p);
    let calls = d.on(
        reply(id_of(&calls[0]), r#"{"mode":"edit","reply":"好"}"#),
        &p,
    );
    assert_eq!(d.phase(), &Phase::Editing);
    let calls = d.on(
        reply(
            id_of(&calls[0]),
            r#"{"edits":[{"type":"set_transition","clip":"b","transition":{"kind":"nope","duration":0.3}}]}"#,
        ),
        &p,
    );
    let calls = d.on(
        Input::Applied {
            id: id_of(&calls[0]),
            result: Err("motion_not_found".into()),
        },
        &p,
    );
    let Call::Language { messages, .. } = &calls[0] else {
        panic!("{calls:?}")
    };
    assert!(messages
        .last()
        .unwrap()
        .content
        .contains("motion_not_found"));
    let calls = d.on(
        reply(
            id_of(&calls[0]),
            r#"{"edits":[{"type":"set_transition","clip":"b","transition":{"kind":"nope2","duration":0.3}}]}"#,
        ),
        &p,
    );
    let calls = d.on(
        Input::Applied {
            id: id_of(&calls[0]),
            result: Err("motion_not_found".into()),
        },
        &p,
    );
    assert!(calls.is_empty());
    assert_eq!(d.phase(), &Phase::Approval);
}

#[test]
fn applied_edits_are_shown_to_the_user_without_a_review() {
    let p = project();
    let (mut d, calls) = Director::start("t".into(), "第二段右移一点".into(), vec![style()], &p);
    let calls = d.on(
        reply(id_of(&calls[0]), r#"{"mode":"edit","reply":"好"}"#),
        &p,
    );
    let calls = d.on(
        reply(
            id_of(&calls[0]),
            r#"{"edits":[{"type":"set_transform","clip":"b","transform":{"x":0.05}}]}"#,
        ),
        &p,
    );
    let Call::Apply { id, edits } = &calls[0] else {
        panic!("{calls:?}")
    };
    assert!(
        matches!(&edits[0], ControlEdit::SetTransform { transform, .. }
        if transform.x == 0.05 && transform.scale == 1.0)
    );
    // Stale results are ignored.
    assert!(d.on(reply("t-ask-1".into(), "{}"), &p).is_empty());
    let calls = d.on(
        Input::Applied {
            id: id.clone(),
            result: Ok(1),
        },
        &p,
    );
    let calls = d.on(frames_for(&calls[0]), &p);
    assert!(calls.is_empty(), "no vision call when editing");
    assert_eq!(d.approval().unwrap().frames.len(), 2);
}

#[test]
fn unknown_style_fails_instead_of_guessing() {
    let p = project();
    let (mut d, calls) = Director::start("t".into(), "做个视频".into(), vec![style()], &p);
    d.on(
        reply(
            id_of(&calls[0]),
            r#"{"mode":"produce","extension":"nope","params":{}}"#,
        ),
        &p,
    );
    assert!(matches!(d.phase(), Phase::Failed { .. }));
}

#[test]
fn picture_swaps_written_as_edits_and_bad_edits_do_not_sink_the_plan() {
    let p = project();
    let plan = prompts::parse_edits(
        r#"{"edits":[
            {"type":"set_transform","clip":"a","transform":{"x":0.1}},
            {"type":"pictures","clip":"b","draw":"a burnt burger"},
            {"type":"spin","clip":"a"}
        ]}"#,
        Some(&p),
    )
    .unwrap();
    assert_eq!(plan.edits.len(), 1);
    assert_eq!(plan.pictures.len(), 1);
    assert_eq!(plan.skipped.len(), 1);
}

#[test]
fn picture_swaps_are_fetched_then_applied_with_the_edits() {
    let p = project();
    let (mut d, calls) = Director::start("t".into(), "换掉第二段的画面".into(), vec![style()], &p);
    let calls = d.on(
        reply(id_of(&calls[0]), r#"{"mode":"edit","reply":"好"}"#),
        &p,
    );
    let calls = d.on(
        reply(
            id_of(&calls[0]),
            r#"{"edits":[{"type":"set_transform","clip":"a","transform":{"x":0.1}}],
                "pictures":[{"clip":"b","library":"krabs-shock"},
                            {"clip":"a","draw":"a burnt burger with smoke"},
                            {"clip":"b","library":"nope"}]}"#,
        ),
        &p,
    );
    let Call::Picture { id, source } = &calls[0] else {
        panic!("{calls:?}")
    };
    assert_eq!(
        source,
        &PictureSource::File("D:/lib/krabs-shock.png".into())
    );
    let calls = d.on(
        Input::Picture {
            id: id.clone(),
            result: Ok("m-new".into()),
        },
        &p,
    );
    let Call::Picture { id, source } = &calls[0] else {
        panic!("{calls:?}")
    };
    assert!(matches!(source, PictureSource::Draw(prompt) if prompt.contains("burnt")));
    let calls = d.on(
        Input::Picture {
            id: id.clone(),
            result: Err("content_policy".into()),
        },
        &p,
    );
    let Call::Apply { edits, .. } = &calls[0] else {
        panic!("{calls:?}")
    };
    assert_eq!(
        edits.len(),
        2,
        "one swap that worked plus the transform: {edits:?}"
    );
    assert!(matches!(&edits[0], ControlEdit::ReplaceMedia { media_id, .. } if media_id == "m-new"));
    assert!(d
        .log()
        .iter()
        .any(|e| e.kind == LogKind::Problem && e.text.contains("nope")));
}
