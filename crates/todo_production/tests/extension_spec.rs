//! The extension standard as code: manifests, parameters, protocol, items.
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use todo_workflow::workflow::graph::{Graph, GRAPH_SPEC};
use todo_production::items::{validate_items, Item};
use todo_workflow::workflow::manifest::{Kind, Manifest, Service};
use todo_workflow::workflow::protocol::{
    HostMessage, ServiceCall, ServiceErrorClass, WorkerMessage,
};

fn package(id: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../extensions/builtin")
        .join(id)
}

fn read_in(dir: PathBuf) -> impl Fn(&str) -> Option<String> {
    move |path: &str| std::fs::read_to_string(dir.join(path)).ok()
}

fn problems(manifest: &Value) -> Vec<String> {
    let manifest: Manifest = serde_json::from_value(manifest.clone()).expect("parses");
    let files = |path: &str| match path {
        "prompts/planner.md" => Some("你是分镜师。".to_owned()),
        "prompts/asset.md" => Some("Draw {{draw}} as {{role}} in {{palette}}.".to_owned()),
        "prompts/bad.md" => Some("no placeholder".to_owned()),
        _ => None,
    };
    manifest
        .validate(&files)
        .into_iter()
        .map(|p| p.to_string())
        .collect()
}

/// A code-generated animation: no script, no AI, frames encoded by the host.
fn animation() -> Value {
    json!({
        "spec": "todo-extension/1",
        "id": "studio.title-burst",
        "name": "片头爆字",
        "version": "1.0.0",
        "kind": "workflow",
        "params": [
            {"type": "text", "id": "title", "label": "标题", "group": "text", "default": "TO DO", "maxLength": 20},
            {"type": "number", "id": "seconds", "label": "时长", "group": "timing", "default": 3, "min": 1, "max": 10, "step": 0.5, "unit": "秒"},
            {"type": "bool", "id": "transparent", "label": "透明底", "group": "output", "default": true}
        ],
        "requires": {"services": ["encode"], "runtime": {"type": "native"}},
        "worker": {"entry": "worker/title_burst.exe", "ops": ["run"], "timeoutSec": 120}
    })
}

#[test]
fn the_chalk_hand_package_is_valid() {
    let dir = package("chalk-hand");
    let text = std::fs::read_to_string(dir.join("extension.json")).unwrap();
    let manifest: Manifest = serde_json::from_str(&text).unwrap();
    assert_eq!(manifest.kind, Kind::Narration);
    let found = manifest.validate(&read_in(dir));
    assert!(found.is_empty(), "{found:?}");
    let params = manifest.resolve_params(&BTreeMap::new()).unwrap();
    assert_eq!(params["voice"], "zh_female_mizai_uranus_bigtts");
    assert_eq!(params["subtitles"], true);
    assert!(manifest.prompt_vars(&params)["palette"].contains("chalk"));
}

#[test]
fn the_spongebob_package_declares_a_valid_graph() {
    let dir = package("spongebob-koubo");
    let text = std::fs::read_to_string(dir.join("extension.json")).unwrap();
    let manifest: Manifest = serde_json::from_str(&text).unwrap();
    assert_eq!(manifest.kind, Kind::Workflow);
    assert_eq!(manifest.graph.as_deref(), Some("graph.json"));
    let found = manifest.validate(&read_in(dir.clone()));
    assert!(found.is_empty(), "{found:?}");
    let graph_text = std::fs::read_to_string(dir.join("graph.json")).unwrap();
    let graph: Graph = serde_json::from_str(&graph_text).unwrap();
    assert_eq!(graph.spec, GRAPH_SPEC);
    let order = graph.topo_order().expect("DAG");
    assert!(
        order.first().is_some_and(|id| *id == "script" || *id == "media_bin"),
        "sources first: {order:?}"
    );
    assert_eq!(order.last().copied(), Some("done"));
    assert!(order.iter().any(|id| *id == "script"));
    assert!(order.iter().any(|id| *id == "picture_batch"));
}

#[test]
fn a_workflow_graph_must_be_a_dag() {
    let mut m = animation();
    m["graph"] = json!("graph.json");
    let files = |path: &str| match path {
        "graph.json" => Some(
            json!({
                "spec": "todo-graph/1",
                "nodes": [
                    {"id": "a", "type": "emit"},
                    {"id": "b", "type": "emit"}
                ],
                "edges": [
                    {"from": "a", "to": "b", "out": "items", "in": "payload"},
                    {"from": "b", "to": "a", "out": "items", "in": "payload"}
                ]
            })
            .to_string(),
        ),
        _ => None,
    };
    let manifest: Manifest = serde_json::from_value(m).unwrap();
    let found = manifest.validate(&files);
    assert!(
        found.iter().any(|p| p.path.starts_with("graph") && p.message.contains("环")),
        "{found:?}"
    );
}

#[test]
fn extensions_need_neither_input_nor_ai() {
    assert!(
        problems(&animation()).is_empty(),
        "{:?}",
        problems(&animation())
    );
    let mut media = animation();
    media["input"] = json!({"media": {"label": "要加特效的片段", "kinds": ["video", "image"], "min": 1, "max": 4}});
    media["requires"]["services"] = json!([]);
    media["requires"]["runtime"] = json!({"type": "python", "version": ">=3.12"});
    media["worker"]["entry"] = json!("worker/main.py");
    assert!(problems(&media).is_empty(), "{:?}", problems(&media));
}

#[test]
fn mistakes_are_reported_precisely() {
    let cases: Vec<(Value, &str)> = vec![
        (
            {
                let mut m = animation();
                m["id"] = json!("Title Burst");
                m
            },
            "id",
        ),
        (
            {
                let mut m = animation();
                m["version"] = json!("1.0");
                m
            },
            "version",
        ),
        (
            {
                let mut m = animation();
                m["worker"]["ops"] = json!(["render"]);
                m
            },
            "必须提供 run",
        ),
        (
            {
                let mut m = animation();
                m["requires"] = json!({"services": []});
                m
            },
            "必须声明运行时",
        ),
        (
            {
                let mut m = animation();
                m["worker"]["entry"] = json!("../evil.exe");
                m
            },
            "worker.entry",
        ),
        (
            {
                let mut m = animation();
                m["budget"] = json!({"image": 5});
                m
            },
            "budget",
        ),
        (
            {
                let mut m = animation();
                m["quick"] = json!(["missing"]);
                m
            },
            "quick",
        ),
        (
            {
                let mut m = animation();
                m["params"][1]["default"] = json!(99);
                m
            },
            "默认值越界",
        ),
        (
            {
                let mut m = animation();
                m["params"][0]["when"] = json!({"seconds": ["3"]});
                m
            },
            "when 只能依赖",
        ),
        (
            {
                let mut m = animation();
                m["params"].as_array_mut().unwrap().push(json!({"type": "bool", "id": "title", "label": "x", "group": "other", "default": false}));
                m
            },
            "重复",
        ),
        (
            {
                let mut m = animation();
                m["kind"] = json!("narration");
                m
            },
            "kind = narration 时必需",
        ),
    ];
    for (manifest, expected) in cases {
        let found = problems(&manifest);
        assert!(
            found.iter().any(|p| p.contains(expected)),
            "expected {expected:?} in {found:?}"
        );
    }
    let narration = json!({
        "spec": "todo-extension/1", "id": "a.b", "name": "n", "version": "1.0.0", "kind": "narration",
        "input": {"script": {"label": "文案", "maxLength": 3000}},
        "requires": {"services": ["chat"]},
        "narration": {
            "planner": {"prompt": "prompts/missing.md", "maxElements": 4},
            "assets": {"template": "prompts/bad.md", "roles": {"hero": "h"}, "size": "big"},
            "segment": {"target": 120, "max": 200}, "boardSec": 10, "bake": "bake_board",
            "pacing": {"transition": 0.4, "wobble": 1}
        }
    });
    let found = problems(&narration).join("\n");
    for expected in [
        "Image",
        "Speech",
        "找不到提示词",
        "{{draw}}",
        "缺少 support",
        "WxH",
        "没有操作 bake_board",
        "未知字段 wobble",
    ] {
        assert!(
            found.contains(expected),
            "expected {expected:?} in\n{found}"
        );
    }
    // Unknown fields fail to parse instead of being ignored.
    let mut typo = animation();
    typo["worker"]["timeoutSecs"] = json!(5);
    assert!(serde_json::from_value::<Manifest>(typo).is_err());
}

#[test]
fn parameters_resolve_strictly() {
    let manifest: Manifest = serde_json::from_value(animation()).unwrap();
    let resolve = |given: Value| manifest.resolve_params(&serde_json::from_value(given).unwrap());
    let defaults = resolve(json!({})).unwrap();
    assert_eq!(defaults["seconds"], 3.0);
    assert_eq!(defaults["transparent"], true);
    assert_eq!(
        resolve(json!({"seconds": 4.3})).unwrap()["seconds"],
        4.5,
        "snapped to step"
    );
    assert!(resolve(json!({"seconds": 11}))
        .unwrap_err()
        .to_string()
        .contains("1~10"));
    assert!(resolve(json!({"nope": 1}))
        .unwrap_err()
        .to_string()
        .contains("未声明"));
    assert!(resolve(json!({"title": "x".repeat(21)})).is_err());
    assert!(resolve(json!({"transparent": "yes"})).is_err());
}

#[test]
fn protocol_messages_round_trip() {
    let lines = [
        r#"{"type":"hello","spec":"todo-worker/1","host":"0.2.0","package":"p"}"#,
        r#"{"type":"run","id":"r1","op":"run","input":{},"params":{"seconds":3},"canvas":{"width":1920,"height":1080,"fps":30.0},"workdir":"w","seed":7}"#,
        r#"{"type":"reply","to":"c1","ok":false,"error":{"class":"content_policy","message":"refused"}}"#,
        r#"{"type":"cancel","id":"r1"}"#,
        r#"{"type":"shutdown"}"#,
    ];
    for line in lines {
        let message: HostMessage =
            serde_json::from_str(line).unwrap_or_else(|e| panic!("{line}: {e}"));
        assert_eq!(
            serde_json::to_value(&message).unwrap(),
            serde_json::from_str::<Value>(line).unwrap()
        );
    }
    let worker = [
        r#"{"type":"ready","spec":"todo-worker/1","ops":["run"],"version":"1.0.0"}"#,
        r#"{"type":"call","id":"c1","run":"r1","service":"encode","request":{"frames":"frames/title","fps":30,"alpha":true}}"#,
        r#"{"type":"progress","run":"r1","phase":"渲染","fraction":0.5}"#,
        r#"{"type":"done","run":"r1","result":{}}"#,
        r#"{"type":"failed","run":"r1","error":{"class":"invalid_input","message":"empty title"}}"#,
    ];
    for line in worker {
        let message: WorkerMessage =
            serde_json::from_str(line).unwrap_or_else(|e| panic!("{line}: {e}"));
        if let WorkerMessage::Call {
            service, request, ..
        } = &message
        {
            assert!(matches!(
                ServiceCall::parse(*service, request).unwrap(),
                ServiceCall::Encode(_)
            ));
        }
    }
    assert!(
        serde_json::from_str::<WorkerMessage>(r#"{"type":"done","run":"r1","extra":1}"#).is_err()
    );
    let bad = ServiceCall::parse(
        Service::Chat,
        &json!({"messages": [{"role": "user", "content": "x"}], "temperature": 3}),
    )
    .unwrap_err();
    assert_eq!(bad.class, ServiceErrorClass::Invalid);
    assert!(ServiceCall::parse(Service::Encode, &json!({"frames": "../x", "fps": 30})).is_err());
}

#[test]
fn items_are_checked_before_they_reach_the_timeline() {
    let files: BTreeSet<&str> = ["out/title.mp4", "out/voice.wav"].into();
    let exists = |path: &str| files.contains(path);
    let item = |value: Value| -> Item { serde_json::from_value(value).unwrap() };
    let voice = item(
        json!({"key": "voice", "rev": "a1", "role": "voice", "start": 0, "duration": 3,
        "source": {"type": "file", "file": "out/voice.wav"}, "gain": 0.8}),
    );
    let title = item(
        json!({"key": "title/main", "rev": "b2", "role": "visual", "lane": 1, "start": 0.5, "duration": 2.5,
        "source": {"type": "file", "file": "out/title.mp4"}, "transform": {"scale": 0.8}, "intro": {"kind": "fade", "duration": 0.3}, "link": "voice"}),
    );
    assert!(validate_items(&[voice.clone(), title.clone()], &BTreeSet::new(), &exists).is_ok());
    let fail = |items: &[Item]| validate_items(items, &BTreeSet::new(), &exists).unwrap_err();
    assert!(fail(&[voice.clone(), voice.clone()]).contains("重复"));
    let mut missing = title.clone();
    missing.source =
        serde_json::from_value(json!({"type": "file", "file": "out/none.png"})).unwrap();
    assert!(fail(&[voice.clone(), missing]).contains("不存在"));
    let mut orphan = title.clone();
    orphan.link = Some("ghost".into());
    assert!(fail(&[orphan]).contains("ghost"));
    let mut timeless = voice.clone();
    timeless.duration = f64::NAN;
    assert!(fail(&[timeless]).contains("duration"));
    let mut loud_video = title;
    loud_video.gain = Some(1.0);
    assert!(fail(&[voice, loud_video]).contains("gain"));
}

#[test]
fn the_chalk_hand_recipe_drives_the_pipeline() {
    use todo_workflow::workflow::{narration_spec, HostDefaults};
    use todo_production::production::SplitMode;
    let dir = package("chalk-hand");
    let manifest: Manifest =
        serde_json::from_str(&std::fs::read_to_string(dir.join("extension.json")).unwrap())
            .unwrap();
    let host = HostDefaults {
        audio_model: "seed-tts-2.0".into(),
        speaker: "zh_female_mizai_uranus_bigtts".into(),
        sample_rate: 24000,
    };
    let given: BTreeMap<String, Value> = serde_json::from_value(
        json!({"voice": "S_abc123", "split": "paragraph", "subtitles": false, "palette": "mono"}),
    )
    .unwrap();
    let params = manifest.resolve_params(&given).unwrap();
    let (spec, split, limits) = narration_spec(&manifest, &params, &read_in(dir), &host).unwrap();
    assert_eq!(split, SplitMode::Paragraph);
    assert_eq!(
        spec.voice.resource_id, "seed-icl-2.0",
        "cloned voices use the cloning resource"
    );
    assert_eq!(spec.subtitle_chars, None);
    assert_eq!(spec.max_elements, 4);
    assert!(spec.planner_system.contains("分镜师"));
    assert!(spec.asset.template.contains("{{draw}}"));
    assert_eq!(
        spec.asset.vars["palette"],
        "white chalk only, no colored chalk"
    );
    assert_eq!(limits.max, 200);
}
