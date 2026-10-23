use serde::Deserialize;

#[derive(Deserialize)]
struct Corpus {
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    id: String,
    compile_valid: bool,
    source: String,
}

#[test]
fn shared_expression_and_local_corpus_matches_compiler_admission() {
    let corpus: Corpus = serde_json::from_str(include_str!(
        "../../../docs/burn-check/tests/expression-parity.json"
    ))
    .unwrap();
    for case in corpus.cases {
        let result = antiburn_checks::compile(&case.source);
        assert_eq!(
            result.is_ok(),
            case.compile_valid,
            "{}: {result:?}",
            case.id
        );
    }
}
