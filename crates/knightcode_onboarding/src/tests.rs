use super::*;
use gpui::TestAppContext;
use http_client::{AsyncBody, FakeHttpClient, Response};
use knightcode_engine::{Endpoint, Engine};
use std::sync::Mutex;

const NO_ACCOUNTS: &str = r#"{"accounts":[],"loginOptions":[{"providerId":"anthropic","providerName":"Anthropic","type":"oauth","label":"Anthropic (Claude Pro/Max)","isSubscription":true},{"providerId":"openai","providerName":"OpenAI","type":"api_key","label":"OpenAI API key","isSubscription":false}]}"#;
const ONE_ACCOUNT: &str = r#"{"accounts":[{"providerId":"anthropic","providerName":"Anthropic","type":"oauth","isSubscription":true}],"loginOptions":[]}"#;
const NO_MODELS: &str = r#"{"models":[],"default":null}"#;
const NO_CHOICE: &str = r#"{"models":[{"ref":"anthropic/claude-opus-5","id":"claude-opus-5","providerId":"anthropic","providerName":"Anthropic","name":"Claude Opus 5","contextWindow":200000,"maxTokens":32000,"reasoning":true,"input":["text"],"cost":{}}],"default":null}"#;
const CHOSEN: &str = r#"{"models":[{"ref":"anthropic/claude-opus-5","id":"claude-opus-5","providerId":"anthropic","providerName":"Anthropic","name":"Claude Opus 5","contextWindow":200000,"maxTokens":32000,"reasoning":true,"input":["text"],"cost":{}}],"default":"anthropic/claude-opus-5"}"#;

fn engine(
    cx: &mut TestAppContext,
    accounts: &'static str,
    models: &'static str,
) -> gpui::Entity<Engine> {
    let bodies = Arc::new(Mutex::new((accounts, models)));
    let http = FakeHttpClient::create(move |request| {
        let bodies = bodies.clone();
        let path = request.uri().path().to_string();
        async move {
            let (accounts, models) = *bodies.lock().unwrap();
            let body = match path.as_str() {
                "/v1/accounts" => accounts,
                "/v1/models" => models,
                "/v1/settings/telemetry" => r#"{"enabled":true,"overridden":false}"#,
                _ => "{}",
            };
            Ok(Response::builder()
                .status(200)
                .body(AsyncBody::from(body))
                .unwrap())
        }
    });
    cx.update(|cx| {
        let settings_store = settings::SettingsStore::test(cx);
        cx.set_global(settings_store);
        cx.set_http_client(http.clone());
    });
    let endpoint = Endpoint {
        url: "http://127.0.0.1:1".into(),
        token: "t".into(),
    };
    cx.update(|cx| Engine::ready_for_tests(endpoint, http, cx))
}

/// The item without a workspace: every assertion here is about which step is
/// reachable, which is the part a walk cannot check cheaply.
async fn first_run(
    cx: &mut TestAppContext,
    accounts: &'static str,
    models: &'static str,
) -> gpui::Entity<FirstRun> {
    let _engine = engine(cx, accounts, models);
    let item = cx.update(|cx| cx.new(|cx| FirstRun::for_tests(cx)));
    cx.run_until_parked();
    item
}

#[gpui::test]
async fn a_signed_in_machine_with_a_model_skips_both_middle_steps(cx: &mut TestAppContext) {
    let item = first_run(cx, ONE_ACCOUNT, CHOSEN).await;
    cx.read(|cx| {
        let run = item.read(cx);
        assert_eq!(run.step, Step::Welcome);
        assert!(run.satisfied(Step::SignIn, cx), "the CLI's login is ours");
        assert!(run.satisfied(Step::Model, cx));
    });
    cx.update(|cx| {
        item.update(cx, |run, cx| {
            run.advance_for_tests(cx);
            assert_eq!(run.step, Step::Open, "nothing left to ask");
        })
    });
}

#[gpui::test]
async fn without_an_account_sign_in_is_a_gate_and_offers_the_engines_options(
    cx: &mut TestAppContext,
) {
    let item = first_run(cx, NO_ACCOUNTS, NO_MODELS).await;
    cx.update(|cx| {
        item.update(cx, |run, cx| {
            run.advance_for_tests(cx);
            assert_eq!(run.step, Step::SignIn);
            assert!(!run.can_advance(cx), "Continue stays disabled");
            // One button per login option, in the engine's order, with no table
            // of providers anywhere in the fork.
            let options = run
                .state
                .as_ref()
                .unwrap()
                .read(cx)
                .login_options
                .iter()
                .map(|option| option.provider_id.clone())
                .collect::<Vec<_>>();
            assert_eq!(options, vec!["anthropic", "openai"]);
        })
    });
}

#[gpui::test]
async fn with_an_account_but_no_model_the_model_step_is_a_gate(cx: &mut TestAppContext) {
    let item = first_run(cx, ONE_ACCOUNT, NO_CHOICE).await;
    cx.update(|cx| {
        item.update(cx, |run, cx| {
            run.advance_for_tests(cx);
            assert_eq!(run.step, Step::Model, "sign-in was satisfied, this is not");
            assert!(!run.can_advance(cx));
        })
    });
}

#[gpui::test]
async fn finishing_writes_first_open(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_global(db::AppDatabase::test_new()));
    let item = first_run(cx, ONE_ACCOUNT, CHOSEN).await;
    cx.update(|cx| {
        assert!(
            matches!(KeyValueStore::global(cx).read_kvp(FIRST_OPEN), Ok(None)),
            "not written before the user finishes"
        );
        item.update(cx, |run, cx| run.finish(cx));
    });
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            KeyValueStore::global(cx).read_kvp(FIRST_OPEN).unwrap(),
            Some("false".to_string())
        );
    });
}
