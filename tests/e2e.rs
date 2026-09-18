//! End-to-end tests: the full command pipeline against the in-process mock
//! server (no network, no API key).

mod common;

use jev_cli::input::{read_text_source, StateInput};
use jev_cli::run::{
    run_ask, run_choose, run_models, run_predict, run_score, AskOptions, ChooseOptions,
    PredictOptions, ScoreOptions, EXIT_NOT_MET, EXIT_OK,
};
use jev_cli::{Error, OutputFormat};

use common::{mock_server, test_client};

fn state(text: &str) -> StateInput {
    StateInput {
        state: Some(text.into()),
        ..Default::default()
    }
}

fn predict_opts(question: &str, threshold: Option<f64>, state: StateInput) -> PredictOptions {
    PredictOptions {
        question: question.into(),
        yes: None,
        no: None,
        threshold,
        state,
        format: OutputFormat::Json,
        model: None,
    }
}

#[tokio::test]
async fn predict_reports_probability_and_passes_threshold() {
    let client = test_client(&mock_server().await);
    let opts = predict_opts(
        "Will this plan fail?",
        Some(0.8),
        state("this plan is high-risk and untested"),
    );
    let outcome = run_predict(&client, &opts).await.unwrap();
    assert_eq!(outcome.code, EXIT_OK);
    let v: serde_json::Value = serde_json::from_str(&outcome.stdout).unwrap();
    assert_eq!(v["type"], "noul");
    assert_eq!(v["probability"], 0.93);
    assert_eq!(v["threshold"], 0.8);
    assert_eq!(v["pass"], true);
    assert!(v["usage"]["input_tokens"].is_number());
}

#[tokio::test]
async fn predict_below_threshold_exits_1_but_still_prints() {
    let client = test_client(&mock_server().await);
    let opts = predict_opts(
        "Will this fail?",
        Some(0.8),
        state("a routine, safe change"),
    );
    let outcome = run_predict(&client, &opts).await.unwrap();
    assert_eq!(outcome.code, EXIT_NOT_MET);
    let v: serde_json::Value = serde_json::from_str(&outcome.stdout).unwrap();
    assert_eq!(v["probability"], 0.07);
    assert_eq!(v["pass"], false);

    // Without a gate, the same low probability is a clean exit 0.
    let opts = predict_opts("Will this fail?", None, state("a routine, safe change"));
    let outcome = run_predict(&client, &opts).await.unwrap();
    assert_eq!(outcome.code, EXIT_OK);
}

#[tokio::test]
async fn predict_text_format_is_the_bare_value() {
    let client = test_client(&mock_server().await);
    let mut opts = predict_opts("Will this fail?", None, state("high-risk"));
    opts.format = OutputFormat::Text;
    let outcome = run_predict(&client, &opts).await.unwrap();
    assert_eq!(outcome.stdout, "0.93\n");
}

#[tokio::test]
async fn predict_echoes_the_requested_model() {
    let client = test_client(&mock_server().await);
    let mut opts = predict_opts("q", None, state("anything"));
    opts.model = Some("mock-jev-small".into());
    let outcome = run_predict(&client, &opts).await.unwrap();
    let v: serde_json::Value = serde_json::from_str(&outcome.stdout).unwrap();
    assert_eq!(v["model"], "mock-jev-small");
}

#[tokio::test]
async fn choose_picks_the_option_named_in_state() {
    let client = test_client(&mock_server().await);
    let opts = ChooseOptions {
        question: "Which approach?".into(),
        options: vec![
            "retry=Retry with backoff".into(),
            "fallback=Use the cache".into(),
            "abort=Stop and ask".into(),
        ],
        min_confidence: None,
        state: state("the cache is warm, so the fallback path looks best"),
        format: OutputFormat::Json,
        model: None,
    };
    let outcome = run_choose(&client, &opts).await.unwrap();
    assert_eq!(outcome.code, EXIT_OK);
    let v: serde_json::Value = serde_json::from_str(&outcome.stdout).unwrap();
    assert_eq!(v["type"], "choice");
    assert_eq!(v["choice"], "fallback");
    assert_eq!(v["probabilities"]["fallback"], 0.9);
    assert_eq!(v["confidence"], 0.88);
}

#[tokio::test]
async fn choose_text_format_is_the_bare_option() {
    let client = test_client(&mock_server().await);
    let opts = ChooseOptions {
        question: "Which approach?".into(),
        options: vec!["retry".into(), "abort".into()],
        min_confidence: None,
        state: state("a transient error; retry is safest"),
        format: OutputFormat::Text,
        model: None,
    };
    let outcome = run_choose(&client, &opts).await.unwrap();
    assert_eq!(outcome.stdout, "retry\n");
}

#[tokio::test]
async fn choose_min_confidence_gates_the_exit_code() {
    let client = test_client(&mock_server().await);
    let opts = |min: Option<f64>| ChooseOptions {
        question: "q".into(),
        options: vec!["a".into(), "b".into()],
        min_confidence: min,
        state: state("anything"),
        format: OutputFormat::Json,
        model: None,
    };
    // The mock answers with confidence 0.88.
    let outcome = run_choose(&client, &opts(Some(0.95))).await.unwrap();
    assert_eq!(outcome.code, EXIT_NOT_MET);
    let v: serde_json::Value = serde_json::from_str(&outcome.stdout).unwrap();
    assert_eq!(v["pass"], false);

    let outcome = run_choose(&client, &opts(Some(0.5))).await.unwrap();
    assert_eq!(outcome.code, EXIT_OK);
}

#[tokio::test]
async fn score_reports_score_and_legend() {
    let client = test_client(&mock_server().await);
    let opts = ScoreOptions {
        question: "How risky is this plan?".into(),
        levels: vec!["trivial".into(), "moderate".into(), "severe".into()],
        state: state("a high-risk migration"),
        format: OutputFormat::Json,
        model: None,
    };
    let outcome = run_score(&client, &opts).await.unwrap();
    assert_eq!(outcome.code, EXIT_OK);
    let v: serde_json::Value = serde_json::from_str(&outcome.stdout).unwrap();
    assert_eq!(v["type"], "score");
    assert_eq!(v["score"], 1.9);
    assert_eq!(v["legend"]["2"], "severe");
}

#[tokio::test]
async fn ask_round_trips_a_full_document() {
    let dir = std::env::temp_dir();
    let path = dir.join("jev-cli-e2e-ask.json");
    std::fs::write(
        &path,
        r#"{
            "state": "a high-risk deploy on a Friday",
            "questions": {
                "will_fail": {"type": "noul", "instructions": "Will it fail?"},
                "approach": {"type": "choice", "instructions": "What now?",
                             "criteria": {"ship": null, "wait": "Wait for Monday"}},
                "risk": {"type": "score", "instructions": "How risky?",
                         "criteria": ["low", "medium", "high"]}
            },
            "model": "mock-jev-small"
        }"#,
    )
    .unwrap();

    let client = test_client(&mock_server().await);
    let opts = AskOptions {
        request: path.clone(),
        format: OutputFormat::Json,
        model: None,
    };
    let outcome = run_ask(&client, &opts).await.unwrap();
    std::fs::remove_file(path).ok();

    assert_eq!(outcome.code, EXIT_OK);
    let v: serde_json::Value = serde_json::from_str(&outcome.stdout).unwrap();
    assert_eq!(v["model"], "mock-jev-small");
    assert_eq!(v["answers"]["will_fail"]["noul"], 0.93);
    assert_eq!(v["answers"]["approach"]["type"], "choice");
    assert_eq!(v["answers"]["risk"]["score"], 1.9);
    assert!(v["usage"]["output_tokens"].is_number());
    // Question order is preserved in the response.
    let ids: Vec<_> = v["answers"].as_object().unwrap().keys().collect();
    assert_eq!(ids, ["will_fail", "approach", "risk"]);
}

#[tokio::test]
async fn ask_cli_model_overrides_document_model() {
    let dir = std::env::temp_dir();
    let path = dir.join("jev-cli-e2e-ask-model.json");
    std::fs::write(
        &path,
        r#"{"state": "s", "model": "doc-model",
            "questions": {"q": {"type": "noul", "instructions": "i"}}}"#,
    )
    .unwrap();

    let client = test_client(&mock_server().await);
    let opts = AskOptions {
        request: path.clone(),
        format: OutputFormat::Json,
        model: Some("cli-model".into()),
    };
    let outcome = run_ask(&client, &opts).await.unwrap();
    std::fs::remove_file(path).ok();
    let v: serde_json::Value = serde_json::from_str(&outcome.stdout).unwrap();
    assert_eq!(v["model"], "cli-model");
}

#[tokio::test]
async fn ask_rejects_invalid_documents() {
    let dir = std::env::temp_dir();
    let path = dir.join("jev-cli-e2e-ask-bad.json");
    std::fs::write(&path, r#"{"state": "s", "questions": {}, "bogus": 1}"#).unwrap();

    let client = test_client(&mock_server().await);
    let opts = AskOptions {
        request: path.clone(),
        format: OutputFormat::Json,
        model: None,
    };
    let err = run_ask(&client, &opts).await.unwrap_err();
    std::fs::remove_file(path).ok();
    assert!(matches!(err, Error::Usage(_)), "{err}");
}

#[tokio::test]
async fn models_lists_available_models() {
    let client = test_client(&mock_server().await);
    let outcome = run_models(&client, OutputFormat::Json).await.unwrap();
    let v: serde_json::Value = serde_json::from_str(&outcome.stdout).unwrap();
    assert_eq!(v["models"][0]["name"], "mock-jev");
    assert_eq!(v["models"].as_array().unwrap().len(), 2);

    let outcome = run_models(&client, OutputFormat::Text).await.unwrap();
    assert!(outcome.stdout.starts_with("mock-jev\t2026-01-01"));
}

#[tokio::test]
async fn state_from_a_file_and_size_cap() {
    let dir = std::env::temp_dir();
    let path = dir.join("jev-cli-e2e-state.txt");
    std::fs::write(&path, "state from a file, high-risk indeed").unwrap();

    let client = test_client(&mock_server().await);
    let input = StateInput {
        state_file: Some(path.clone()),
        ..Default::default()
    };
    let opts = predict_opts("Will it fail?", None, input);
    let outcome = run_predict(&client, &opts).await.unwrap();
    let v: serde_json::Value = serde_json::from_str(&outcome.stdout).unwrap();
    assert_eq!(v["probability"], 0.93);

    // A file past the cap is an error, not a truncation.
    std::fs::write(&path, "0123456789abcdef0").unwrap(); // 17 bytes > cap of 16
    let err = read_text_source(&path, 16, "state").await.unwrap_err();
    assert!(err.to_string().contains("limit"), "{err}");
    std::fs::remove_file(path).ok();
}

#[tokio::test]
async fn usage_errors_fire_before_any_network() {
    // No client is even reachable here: validation must fail first.
    let client = test_client(&mock_server().await);

    let mut opts = predict_opts("", None, state("s"));
    assert!(run_predict(&client, &opts).await.is_err());

    opts = predict_opts("q", Some(1.5), state("s"));
    assert!(run_predict(&client, &opts).await.is_err());

    opts = predict_opts("q", None, StateInput::default());
    let err = run_predict(&client, &opts).await.unwrap_err();
    assert!(err.to_string().contains("no state"), "{err}");

    let choose = ChooseOptions {
        question: "q".into(),
        options: vec!["only-one".into()],
        min_confidence: None,
        state: state("s"),
        format: OutputFormat::Json,
        model: None,
    };
    assert!(run_choose(&client, &choose).await.is_err());

    let score = ScoreOptions {
        question: "q".into(),
        levels: vec!["only".into()],
        state: state("s"),
        format: OutputFormat::Json,
        model: None,
    };
    assert!(run_score(&client, &score).await.is_err());
}
