//! Live integration tests against the real TypeSafe API.
//!
//! These run automatically when `TYPESAFE_API_KEY` is set, and are skipped
//! otherwise. Assertions are directional only — model versions shift
//! calibration, so never assert exact probabilities.

use jev_cli::run::{
    run_ask, run_choose, run_models, run_predict, run_score, AskOptions, ChooseOptions,
    PredictOptions, ScoreOptions, EXIT_OK,
};
use jev_cli::{OutputFormat, StateInput};
use jev_sdk::TypeSafeClient;

fn client() -> Option<TypeSafeClient> {
    TypeSafeClient::from_env().ok()
}

fn state(text: &str) -> StateInput {
    StateInput {
        state: Some(text.into()),
        ..Default::default()
    }
}

#[tokio::test]
async fn predict_a_clear_outcome() {
    let Some(client) = client() else {
        eprintln!("TYPESAFE_API_KEY not set; skipping live test");
        return;
    };
    let opts = PredictOptions {
        question: "Will this command complete successfully?".into(),
        yes: None,
        no: None,
        threshold: Some(0.5),
        state: state("Running `echo hello` in a healthy shell on a local machine."),
        format: OutputFormat::Json,
        model: None,
    };
    let outcome = run_predict(&client, &opts).await.expect("predict failed");
    assert_eq!(outcome.code, EXIT_OK, "{}", outcome.stdout);
    let v: serde_json::Value = serde_json::from_str(&outcome.stdout).unwrap();
    let p = v["probability"].as_f64().unwrap();
    assert!(p > 0.5, "echo hello clearly succeeds, got {p}");
}

#[tokio::test]
async fn choose_between_two_paths() {
    let Some(client) = client() else {
        eprintln!("TYPESAFE_API_KEY not set; skipping live test");
        return;
    };
    let opts = ChooseOptions {
        question: "Which approach should the agent take?".into(),
        options: vec![
            "retry=Retry the request with exponential backoff".into(),
            "abort=Give up immediately and never retry".into(),
        ],
        min_confidence: None,
        state: state(
            "The HTTP request failed with a 503 rate-limit response and a Retry-After \
             header of 2 seconds. The operation is important and idempotent.",
        ),
        format: OutputFormat::Json,
        model: None,
    };
    let outcome = run_choose(&client, &opts).await.expect("choose failed");
    let v: serde_json::Value = serde_json::from_str(&outcome.stdout).unwrap();
    let choice = v["choice"].as_str().unwrap();
    assert!(["retry", "abort"].contains(&choice), "unexpected: {choice}");
    assert_eq!(
        choice, "retry",
        "a transient 503 on an idempotent op should retry"
    );
    assert!(v["confidence"].as_f64().unwrap() > 0.0);
}

#[tokio::test]
async fn score_a_trivial_action() {
    let Some(client) = client() else {
        eprintln!("TYPESAFE_API_KEY not set; skipping live test");
        return;
    };
    let opts = ScoreOptions {
        question: "How risky is this action?".into(),
        levels: vec!["trivial".into(), "moderate".into(), "severe".into()],
        state: state("Reading the file /etc/hostname to display it to the user."),
        format: OutputFormat::Json,
        model: None,
    };
    let outcome = run_score(&client, &opts).await.expect("score failed");
    let v: serde_json::Value = serde_json::from_str(&outcome.stdout).unwrap();
    let score = v["score"].as_f64().unwrap();
    assert!(
        score < 1.0,
        "reading a hostname file is trivial, got {score}"
    );
}

#[tokio::test]
async fn ask_a_full_document() {
    let Some(client) = client() else {
        eprintln!("TYPESAFE_API_KEY not set; skipping live test");
        return;
    };
    let path = std::env::temp_dir().join("jev-cli-live-ask.json");
    std::fs::write(
        &path,
        r#"{
            "state": "Merging a PR that only fixes a typo in the README; CI is green.",
            "questions": {
                "safe_to_merge": {"type": "noul", "instructions": "Is this PR safe to merge without further review?"}
            }
        }"#,
    )
    .unwrap();
    let opts = AskOptions {
        request: path.clone(),
        format: OutputFormat::Json,
        model: None,
    };
    let outcome = run_ask(&client, &opts).await.expect("ask failed");
    std::fs::remove_file(path).ok();
    let v: serde_json::Value = serde_json::from_str(&outcome.stdout).unwrap();
    let p = v["answers"]["safe_to_merge"]["noul"].as_f64().unwrap();
    assert!(p > 0.5, "a green typo fix is safe, got {p}");
}

#[tokio::test]
async fn models_lists_jev() {
    let Some(client) = client() else {
        eprintln!("TYPESAFE_API_KEY not set; skipping live test");
        return;
    };
    let outcome = run_models(&client, OutputFormat::Json)
        .await
        .expect("models failed");
    let v: serde_json::Value = serde_json::from_str(&outcome.stdout).unwrap();
    let names: Vec<_> = v["models"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|m| m["name"].as_str())
        .collect();
    assert!(
        names.iter().any(|n| n.starts_with("jev")),
        "expected a jev model in {names:?}"
    );
}
