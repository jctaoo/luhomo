use luhomo_core::{config::models::UpdateStrategy, *};
use time::ext::NumericalDuration;

fn main() {
    let source = config::models::ConfigurationSource::remote_url()
        .url("https://baidu.com")
        .update_strategy(UpdateStrategy::builder().auto_update(true).interval(3.hours()).build())
        .use_proxy(true)
        .call().expect("Unknown url");
    // build a configuration item
    let item = config::models::ConfigurationItem::builder()
        .display_name("Hello")
        .source(source)
        .build();

    // test serialization
    let json = serde_json::to_string_pretty(&item).unwrap();

    println!("json: {}", json);

    // test deserialization
    let item2: config::models::ConfigurationItem = serde_json::from_str(&json).unwrap();
    println!("item2: {:?}", item2);
}

