//! Parity with the editor's previous JavaScript rules. The fixture holds the
//! inputs and the old outputs (generated once; see its `generator` field).
use serde_json::Value;
use todo_production::segment::*;
use todo_production::text::*;
use todo_production::ChatRole;

fn fixture() -> Value {
    let bytes = include_bytes!("fixtures/production_golden.json");
    serde_json::from_slice(bytes).unwrap()
}

fn limits(value: &Value) -> Limits {
    serde_json::from_value(value.clone()).unwrap()
}

fn usizes(value: &Value) -> Vec<usize> {
    serde_json::from_value(value.clone()).unwrap()
}

#[test]
fn text_helpers() {
    let golden = fixture();
    for case in golden["textCases"].as_array().unwrap() {
        let input = case["input"].as_str().unwrap();
        assert_eq!(
            clean_script(input),
            case["cleanScript"],
            "cleanScript {input:?}"
        );
        assert_eq!(clean_text(input), case["cleanText"], "cleanText {input:?}");
        assert_eq!(
            char_count(input),
            case["chars"].as_u64().unwrap() as usize,
            "chars {input:?}"
        );
    }
}

#[test]
fn segmentation_matches_the_previous_rules() {
    let golden = fixture();
    let cases = golden["segmentCases"].as_array().unwrap();
    assert!(cases.len() > 20);
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let raw = case["raw"].as_str().unwrap();
        let script = clean_script(raw);
        assert_eq!(script, case["script"], "{name}: clean");
        for (over, expected) in case["units"].as_object().unwrap() {
            let expected: Vec<Unit> = serde_json::from_value(expected.clone()).unwrap();
            let actual = split_units(&script, over.parse().unwrap());
            assert_eq!(actual, expected, "{name}: units over {over}");
        }
        let units = split_units(&script, 108);
        for h in case["heuristic"].as_array().unwrap() {
            let actual = heuristic_starts(
                &units,
                limits(&h["limits"]),
                h["paragraph"].as_bool().unwrap(),
            );
            assert_eq!(actual, usizes(&h["starts"]), "{name}: heuristic {h}");
        }
        for e in case["enforce"].as_array().unwrap() {
            let input: Vec<i64> = serde_json::from_value(e["input"].clone()).unwrap();
            assert_eq!(
                enforce_limits(&units, &input, limits(&e["limits"])),
                usizes(&e["starts"]),
                "{name}: enforce {input:?}"
            );
        }
        for o in case["outline"].as_array().unwrap() {
            let actual = sanitize_outline(&o["raw"], &units, limits(&o["limits"]));
            match o.get("error") {
                Some(error) => assert_eq!(actual.unwrap_err(), error.as_str().unwrap(), "{name}"),
                None => {
                    let actual = actual.unwrap();
                    let expected = &o["out"];
                    assert_eq!(actual.title, expected["title"], "{name}: title");
                    assert_eq!(actual.starts, usizes(&expected["starts"]), "{name}: starts");
                    assert_eq!(
                        serde_json::to_value(&actual.titles).unwrap(),
                        expected["titles"],
                        "{name}: titles"
                    );
                    assert_eq!(
                        serde_json::to_value(&actual.cast).unwrap(),
                        expected["cast"],
                        "{name}: cast"
                    );
                }
            }
        }
        let first = &case["heuristic"][0];
        let texts = segment_texts(&units, &usizes(&first["starts"]));
        assert_eq!(
            serde_json::to_value(&texts).unwrap(),
            case["texts"],
            "{name}: texts"
        );
        let title = if name.starts_with("edge") {
            ""
        } else {
            "测试标题"
        };
        let messages = build_segment_messages(
            &units,
            Limits {
                target: 120,
                max: 200,
            },
            title,
        );
        assert_eq!(messages[0].role, ChatRole::System);
        assert_eq!(
            messages[0].content, case["messages"][0]["content"],
            "{name}: system"
        );
        assert_eq!(
            messages[1].content, case["messages"][1]["content"],
            "{name}: user"
        );
    }
}

#[test]
fn joined_segments_are_the_script() {
    let golden = fixture();
    for case in golden["segmentCases"].as_array().unwrap() {
        let script = clean_script(case["raw"].as_str().unwrap());
        let units = split_units(&script, 108);
        let joined: String = units.iter().map(|u| u.text.as_str()).collect();
        let strip = |s: &str| {
            s.chars()
                .filter(|c| !c.is_whitespace() && *c != '-')
                .collect::<String>()
        };
        assert_eq!(strip(&joined), strip(&script), "{}", case["name"]);
    }
}

mod storyboard_parity {
    use super::fixture;
    use serde_json::Value;
    use std::collections::BTreeMap;
    use todo_production::segment::CastMember;
    use todo_production::storyboard::*;

    fn phrases(case: &Value) -> Vec<Phrase> {
        serde_json::from_value(case["phrases"].clone()).unwrap()
    }

    #[test]
    fn plans_schedules_and_messages() {
        let golden = fixture();
        let max = golden["pack"]["maxElements"].as_u64().unwrap() as usize;
        let system = golden["pack"]["system"].as_str().unwrap();
        let cases = golden["planCases"].as_array().unwrap();
        assert!(cases.len() > 40);
        for case in cases {
            let name = case["name"].as_str().unwrap();
            let phrases = phrases(case);
            let plan = if case.get("fallback").is_some() {
                fallback_plan(max, &phrases)
            } else {
                let max_boards = case["maxScenes"].as_u64().unwrap() as usize;
                sanitize_plan(&case["raw"], max, &phrases, max_boards)
            };
            if let Some(error) = case.get("error") {
                assert_eq!(plan.unwrap_err(), error.as_str().unwrap(), "{name}");
                continue;
            }
            let plan = plan.unwrap_or_else(|e| panic!("{name}: {e}"));
            let expected: BoardPlan = serde_json::from_value(case["plan"].clone()).unwrap();
            assert_eq!(plan, expected, "{name}: plan");
            // Timing deliberately differs from the old hard-cut schedule: see tests/pacing.rs.
            if let Some(messages) = case.get("messages") {
                let cast = [CastMember {
                    name: "小明".into(),
                    look: "cap".into(),
                }];
                let actual = build_board_messages(
                    system,
                    max,
                    &phrases,
                    &BoardPrompt {
                        board_sec: 10.0,
                        max_boards: 6,
                        cast: &cast,
                        title: "全片",
                        segment_title: "本段",
                        previous: "上一段结尾",
                        next: "",
                    },
                );
                // The old prompt plus the breathing rule before the context lines.
                let old = messages[0]["content"].as_str().unwrap();
                let at = old
                    .find(
                        "
全片：",
                    )
                    .unwrap();
                let expected = format!(
                    "{}
{BREATHING_RULE}{}",
                    &old[..at],
                    &old[at..]
                );
                assert_eq!(actual[0].content, expected, "{name}: system");
                assert_eq!(actual[1].content, messages[1]["content"], "{name}: user");
            }
        }
    }

    #[test]
    fn asset_prompts() {
        let golden = fixture();
        let assets = &golden["pack"]["assets"];
        let template = assets["template"].as_str().unwrap();
        let avoid: Vec<String> = serde_json::from_value(assets["avoid"].clone()).unwrap();
        let palette = golden["pack"]["palette"]["options"].as_array().unwrap();
        for case in golden["assetCases"].as_array().unwrap() {
            let element = &case["element"];
            let role = element["role"].as_str().unwrap();
            let chosen = case["params"]["palette"].as_str().unwrap();
            let mut vars = BTreeMap::new();
            vars.insert(
                "draw".to_owned(),
                element["draw"].as_str().unwrap().to_owned(),
            );
            vars.insert(
                "label".to_owned(),
                element["label"].as_str().unwrap().to_owned(),
            );
            vars.insert(
                "role".to_owned(),
                assets["roles"][role].as_str().unwrap().to_owned(),
            );
            let prompt = palette.iter().find(|o| o["value"] == chosen).unwrap()["prompt"]
                .as_str()
                .unwrap();
            vars.insert("palette".to_owned(), prompt.to_owned());
            assert_eq!(asset_prompt(template, &vars, &avoid), case["prompt"]);
        }
    }

    #[test]
    fn to_fixed_rounds_ties_up_on_exact_values() {
        assert_eq!(js_to_fixed(1.25, 1), "1.3");
        assert_eq!(js_to_fixed(0.15, 1), "0.1");
        assert_eq!(js_to_fixed(9.96, 1), "10.0");
        assert_eq!(js_to_fixed(0.0, 1), "0.0");
        assert_eq!(js_to_fixed(12.0, 1), "12.0");
    }
}
