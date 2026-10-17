use todo_production::*;

fn image(prompt: &str, variant: u32) -> TaskSpec {
    TaskSpec {
        provider: Provider::Huniu,
        request: TaskRequest::Image(ImageRequest {
            model: "gpt-image-2".into(),
            prompt: prompt.into(),
            size: "1024x1024".into(),
            quality: None,
            transparent: false,
            variant,
        }),
    }
}

#[test]
fn identical_requests_share_one_key() {
    assert_eq!(
        image("a chalk house", 0).key(),
        image("a chalk house", 0).key()
    );
    let key = image("a chalk house", 0).key();
    assert_eq!(key.as_str().len(), 64);
    assert!(key
        .as_str()
        .bytes()
        .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
}

#[test]
fn any_request_difference_is_a_different_task() {
    let base = image("a chalk house", 0).key();
    assert_ne!(base, image("a chalk house ", 0).key());
    assert_ne!(base, image("a chalk house", 1).key());
    let mut other_provider = image("a chalk house", 0);
    other_provider.provider = Provider::Volc;
    assert_ne!(base, other_provider.key());
}

#[test]
fn keys_are_stable_across_releases() {
    // Changing the encoding silently would re-bill every cached result.
    let speech = TaskSpec {
        provider: Provider::Volc,
        request: TaskRequest::Speech(SpeechRequest {
            text: "复利靠的是时间。".into(),
            speaker: "zh_female_mizai_uranus_bigtts".into(),
            resource_id: "seed-tts-2.0".into(),
            sample_rate: 24000,
        }),
    };
    let json = serde_json::to_string(&speech).unwrap();
    assert_eq!(
        json,
        r#"{"provider":"volc","request":{"capability":"speech","text":"复利靠的是时间。","speaker":"zh_female_mizai_uranus_bigtts","resourceId":"seed-tts-2.0","sampleRate":24000}}"#
    );
    assert_eq!(speech.key(), speech.clone().key());
    assert_eq!(speech.capability(), Capability::Speech);
    assert!(!Capability::Speech.is_paid_per_attempt());
    assert!(Capability::Image.is_paid_per_attempt());
}
