//! A proxy variable must not carry local plain-http requests off the machine (#103). Its own test
//! binary, because it sets process-wide environment variables.

use companion_shell::chat::{HttpService, Service};
use serde_json::json;

#[test]
fn plain_http_requests_never_go_through_a_proxy() {
    let proxy = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    proxy.set_nonblocking(true).unwrap();
    let addr = format!("http://{}", proxy.local_addr().unwrap());
    std::env::remove_var("NO_PROXY");
    std::env::remove_var("no_proxy");
    for var in ["ALL_PROXY", "all_proxy", "HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy"] {
        std::env::set_var(var, &addr);
        for host in ["127.0.0.1", "localhost", "[::1]"] {
            // Port 9 (discard) on this machine: nothing listens, so a direct request just fails.
            let svc = HttpService::new(&format!("http://{host}:9")).unwrap();
            // Without the fix the request waits on the silent proxy, so look from another thread.
            let calls = std::thread::spawn(move || {
                let _ = svc.chat(&json!({ "messages": "SYNTHETIC" }), &mut |_| {});
                let _ = svc.recover(&json!({}));
            });
            while !calls.is_finished() {
                assert!(proxy.accept().is_err(), "{var} sent a request for {host} to the proxy");
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            assert!(proxy.accept().is_err(), "{var} sent a request for {host} to the proxy");
        }
        std::env::remove_var(var);
    }
}
