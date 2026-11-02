use super::*;
use std::collections::VecDeque;

struct Fake {
    replies: VecDeque<Result<Value, Error>>,
    events: VecDeque<Result<Value, Error>>,
    calls: Vec<(String, Value)>,
}

impl Transport for Fake {
    fn request(&mut self, _: u64, method: &str, params: Value, _: &dyn Fn() -> bool) -> Result<Value, Error> {
        self.calls.push((method.into(), params));
        self.replies.pop_front().unwrap_or_else(|| Err(Error::Provider("missing fake response".into())))
    }
    fn next(&mut self, cancelled: &dyn Fn() -> bool) -> Result<Value, Error> {
        if cancelled() {
            return Err(Error::Cancelled);
        }
        self.events.pop_front().unwrap_or(Err(Error::Timeout))
    }
    fn send(&mut self, value: Value) -> Result<(), Error> {
        self.calls.push((value["method"].as_str().unwrap_or_default().to_owned(), value["params"].clone()));
        Ok(())
    }
}

fn fake() -> Fake {
    Fake {
        replies: VecDeque::from([
            Ok(json!({"loginId":"our-login","authUrl":"https://auth.openai.com/authorize?state=test"})),
            Ok(json!({"account":{"type":"chatgpt","planType":"plus","email":"not-retained@example.invalid"}})),
        ]),
        events: VecDeque::from([
            Ok(json!({"method":"account/login/completed","params":{"loginId":"another-login","success":false}})),
            Ok(json!({"method":"account/login/completed","params":{"loginId":"our-login","success":true}})),
        ]),
        calls: Vec::new(),
    }
}

#[test]
fn browser_login_waits_for_its_completion_and_verifies_account() {
    let mut rpc = fake();
    let mut url = None;
    let account = managed_login(
        &mut rpc,
        &mut |value| {
            url = Some(value);
            Ok(())
        },
        &|| false,
    )
    .unwrap();
    assert_eq!(account, AccountInfo::ChatGpt { plan: Some("plus".into()) });
    assert!(url.unwrap().starts_with("https://auth.openai.com/"));
    assert_eq!(rpc.calls, [("account/login/start".into(), json!({"type":"chatgpt"})), ("account/read".into(), json!({"refreshToken":false}))]);
}

#[test]
fn cancellation_timeout_failure_and_untrusted_url_cancel_the_same_login() {
    for mode in 0..5 {
        let mut rpc = fake();
        match mode {
            1 => rpc.events.clear(),
            2 => {
                rpc.events = VecDeque::from([Ok(json!({"method":"account/login/completed","params":{"loginId":"our-login","success":false,"error":"secret"}}))])
            }
            3 => rpc.replies[0] = Ok(json!({"loginId":"our-login","authUrl":"https://auth.openai.com.attacker.invalid/"})),
            4 => rpc.replies[1] = Ok(json!({"account":{"type":"apiKey"}})),
            _ => {}
        }
        let result = managed_login(&mut rpc, &mut |_| Ok(()), &|| mode == 0);
        assert!(result.is_err());
        assert!(!result.unwrap_err().to_string().contains("secret"));
        assert_eq!(rpc.calls.last(), Some(&("account/login/cancel".into(), json!({"loginId":"our-login"}))));
    }
}

#[test]
fn account_info_distinguishes_missing_and_api_login_without_retaining_identity() {
    assert_eq!(account_info(&json!({"account":null})).unwrap(), AccountInfo::SignedOut);
    assert_eq!(account_info(&json!({"account":{"type":"apiKey"}})).unwrap(), AccountInfo::Unsupported);
    assert!(account_info(&json!({})).is_err());
    assert_eq!(
        account_info(&json!({"account":{"type":"chatgpt","planType":"bad\nplan","accessToken":"secret"}})).unwrap(),
        AccountInfo::ChatGpt { plan: None }
    );
}

#[test]
fn only_official_https_login_urls_are_opened() {
    assert!(valid_login_url("https://chatgpt.com/auth/login?state=test"));
    for url in [
        "http://auth.openai.com/",
        "https://auth.openai.com@evil.invalid/",
        "https://evil.invalid/",
        "file:///auth",
        "javascript:alert(1)",
        "https://chatgpt.com/\n",
    ] {
        assert!(!valid_login_url(url));
    }
    assert!(!valid_login_url(&format!("https://chatgpt.com/{}", "a".repeat(8192))));
}
