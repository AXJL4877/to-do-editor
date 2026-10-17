//! Developer helper: copy keys from a `KEY=value` file into the app keychain
//! once (e.g. an old toolkit `.env`). The app never reads such files itself.
//! `cargo run -p todo_ai --example import_env -- <path>`
use todo_ai::secrets::{process_environment, Secret, Secrets};
use todo_production::Provider;

fn main() {
    let path = std::env::args().nth(1).expect("path to a KEY=value file");
    let text = std::fs::read_to_string(&path).expect("readable file");
    let value = |names: &[&str]| {
        text.lines().find_map(|line| {
            let (key, value) = line.trim().split_once('=')?;
            names
                .iter()
                .any(|n| n.eq_ignore_ascii_case(key.trim()))
                .then(|| value.trim().trim_matches(['"', '\'']).to_owned())
        })
    };
    let store = todo_ai::secrets::default_store();
    let secrets = Secrets {
        store: store.as_ref(),
        environment: &process_environment,
    };
    for (provider, names) in [
        (
            Provider::Huniu,
            &["HUNIU_API_KEY", "huniu_api_key", "huniuai_api_key"][..],
        ),
        (
            Provider::Volc,
            &["VOLC_SPEECH_API_KEY", "DOUBAO_SPEECH_API_KEY"][..],
        ),
    ] {
        match value(names).and_then(Secret::new) {
            Some(key) => {
                secrets.save(provider, Some(&key)).expect("keychain write");
                println!("{provider:?}: saved {}", key.masked());
            }
            None => println!("{provider:?}: not found"),
        }
    }
}
