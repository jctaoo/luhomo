use http::HeaderMap;
use luhomo_core::{
    config::models::{ConfigurationItem, ConfigurationSource, UpdateStrategy},
    net::{http::HttpClient, reqwest::ReqwestClient},
};
use time::ext::NumericalDuration;

#[tokio::main]
async fn main() {
    let client = ReqwestClient(reqwest::Client::new());

    let mut headers = HeaderMap::new();
    headers.insert(http::header::ACCEPT, "application/json".parse().unwrap());

    match client.get("https://httpbin.org/json", Some(headers)).await {
        Ok(resp) => println!("GET status: {}, body len: {}", resp.status(), resp.body().len()),
        Err(e) => println!("GET error: {e:?}"),
    }

    let source = ConfigurationSource::remote_url()
        .url("https://example.com")
        .update_strategy(
            UpdateStrategy::builder()
                .auto_update(true)
                .interval(3.hours())
                .build(),
        )
        .use_proxy(true)
        .call()
        .expect("invalid URL");

    let item = ConfigurationItem::builder()
        .display_name("Hello")
        .source(source)
        .build();

    let json = serde_json::to_string_pretty(&item).unwrap();
    println!("json: {}", json);

    let item2: ConfigurationItem = serde_json::from_str(&json).unwrap();
    println!("item2: {:?}", item2);
}
