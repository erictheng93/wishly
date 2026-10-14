//! One mock per integration-test binary; requests are isolated by wishlist slug.
use axum::{http::HeaderMap, routing::post, Json, Router};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex, OnceLock};
use tokio::sync::Notify;
use wishsync_api::config::Config;

#[derive(Clone, Debug)]
pub struct Purge {
    pub authorization: String,
    pub body: Value,
}

#[derive(Default)]
pub struct MockCdn {
    requests: Mutex<Vec<Purge>>,
    received: Notify,
}

pub fn public_url(slug: &str) -> String {
    format!("https://api.example.test/api/v1/public/wishlists/{slug}")
}

pub fn init() -> &'static Arc<MockCdn> {
    static MOCK: OnceLock<Arc<MockCdn>> = OnceLock::new();
    MOCK.get_or_init(|| {
        let seen = Arc::new(MockCdn::default());
        let handler = seen.clone();
        let router = Router::new().route(
            "/zones/zone1/purge_cache",
            post(move |headers: HeaderMap, Json(body): Json<Value>| {
                let seen = handler.clone();
                async move {
                    seen.requests.lock().unwrap().push(Purge {
                        authorization: headers["authorization"].to_str().unwrap().to_owned(),
                        body,
                    });
                    seen.received.notify_waiters();
                    Json(json!({"success":true}))
                }
            }),
        );
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let mut config = Config::dev();
        config.cf_api_base = format!("http://{}", listener.local_addr().unwrap());
        config.cf_zone_id = Some("zone1".into());
        config.cf_purge_token = Some("test-purge-token".into());
        config.api_base_url = "https://api.example.test".into();
        wishsync_api::config::init(config);
        // SQLx tests have independent runtimes; this binary-wide server must outlive them.
        std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async move {
                    axum::serve(tokio::net::TcpListener::from_std(listener).unwrap(), router)
                        .await
                        .unwrap();
                });
        });
        seen
    })
}

impl MockCdn {
    pub fn requests_for(&self, slug: &str) -> Vec<Purge> {
        let url = public_url(slug);
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|request| {
                request.body["files"]
                    .as_array()
                    .is_some_and(|files| files.iter().any(|file| file == &url))
            })
            .cloned()
            .collect()
    }

    pub async fn wait_for(&self, slug: &str) -> Purge {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let received = self.received.notified();
                tokio::pin!(received);
                received.as_mut().enable(); // Register before checking the buffer: no lost wakeups.
                if let Some(request) = self.requests_for(slug).into_iter().next() {
                    return request;
                }
                received.await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("CDN purge missing for wishlist {slug}"))
    }
}
