//! Show the resolved AI settings without spending money or printing keys.
//! `cargo run -p todo_ai --example ai_check [-- <app root>]`
use todo_ai::secrets::{process_environment, Secrets};
use todo_ai::AiConfig;
use todo_production::Provider;

fn main() {
    let root = std::env::args()
        .nth(1)
        .map(Into::into)
        .unwrap_or_else(|| std::env::current_dir().unwrap());
    let config = AiConfig::load(&root).unwrap_or_else(|error| {
        eprintln!("config error: {error}");
        std::process::exit(1);
    });
    let store = todo_ai::secrets::default_store();
    let secrets = Secrets {
        store: store.as_ref(),
        environment: &process_environment,
    };
    let h = &config.huniu;
    println!(
        "huniu  : {} chat={} image={} size={} strategy={:?}",
        h.base_url, h.language_model, h.image_model, h.image_size, h.strategy
    );
    println!(
        "volc   : {} {} speaker={}",
        config.volc.base_url, config.volc.audio_model, config.volc.speaker
    );
    println!("limits : {:?}  proxy={:?}", config.limits, config.proxy);
    for provider in [Provider::Huniu, Provider::Volc] {
        match secrets.resolve(provider) {
            Ok(Some((secret, source))) => {
                println!("{provider:?} key: {} ({source:?})", secret.masked())
            }
            Ok(None) => println!("{provider:?} key: missing (enter it in the AI settings)"),
            Err(error) => println!("{provider:?} key: {error}"),
        }
    }
}
