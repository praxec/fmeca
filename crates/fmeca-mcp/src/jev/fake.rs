//! [`FakeJudge`]: a scripted [`JudgmentModel`] for tests, with no network.
//!
//! Built only for this crate's tests or with the `test-fakes` feature.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use super::{Answer, Decisions, JudgmentModel, Question, Unavailable, UnavailableReason};

/// The model id a [`FakeJudge`] reports.
pub const FAKE_MODEL: &str = "fake/jev";

type AnswerFn = dyn Fn(&str, &Question) -> Answer + Send + Sync;

enum Script {
    Answering(Arc<AnswerFn>),
    Failing(UnavailableReason),
}

/// A judge that answers every question with a closure, or always fails
/// with one reason. It records the questions of every call.
pub struct FakeJudge {
    script: Script,
    calls: Mutex<Vec<BTreeMap<String, Question>>>,
}

impl FakeJudge {
    /// A judge answering each question `(id, question)` with `f`. Its
    /// [`Decisions`] report [`FAKE_MODEL`], no usage and no cost.
    pub fn answering(f: impl Fn(&str, &Question) -> Answer + Send + Sync + 'static) -> FakeJudge {
        Self {
            script: Script::Answering(Arc::new(f)),
            calls: Mutex::new(Vec::new()),
        }
    }

    /// A judge whose every call fails with `reason`.
    pub fn failing(reason: UnavailableReason) -> FakeJudge {
        Self {
            script: Script::Failing(reason),
            calls: Mutex::new(Vec::new()),
        }
    }

    /// The questions of every call so far, oldest first.
    pub fn calls(&self) -> Vec<BTreeMap<String, Question>> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl std::fmt::Debug for FakeJudge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let script = match &self.script {
            Script::Answering(_) => "answering".to_string(),
            Script::Failing(reason) => format!("failing({reason:?})"),
        };
        f.debug_struct("FakeJudge")
            .field("script", &script)
            .finish_non_exhaustive()
    }
}

#[async_trait::async_trait]
impl JudgmentModel for FakeJudge {
    async fn decide(
        &self,
        _state: serde_json::Value,
        questions: BTreeMap<String, Question>,
    ) -> Result<Decisions, Unavailable> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(questions.clone());
        match &self.script {
            Script::Failing(reason) => Err(Unavailable::fixed(*reason, "fake judge failure")),
            Script::Answering(f) => Ok(Decisions {
                answers: questions
                    .iter()
                    .map(|(id, question)| (id.clone(), f(id, question)))
                    .collect(),
                model: FAKE_MODEL.to_string(),
                usage: None,
                cost: None,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn one_question() -> BTreeMap<String, Question> {
        BTreeMap::from([(
            "q".to_string(),
            Question::Noul {
                instructions: json!("applies?"),
                criteria: None,
            },
        )])
    }

    #[tokio::test]
    async fn answering_judge_answers_every_question() {
        let judge = FakeJudge::answering(|_, _| Answer::Noul { noul: 0.25 });
        let decisions = judge.decide(json!({}), one_question()).await;
        assert!(matches!(
            decisions.map(|d| d.answers.get("q").cloned()),
            Ok(Some(Answer::Noul { noul })) if noul == 0.25
        ));
    }

    #[tokio::test]
    async fn failing_judge_reports_its_reason() {
        let judge = FakeJudge::failing(UnavailableReason::Timeout);
        let result = judge.decide(json!({}), one_question()).await;
        assert_eq!(
            result.map(|_| ()).map_err(|e| e.reason),
            Err(UnavailableReason::Timeout)
        );
    }

    #[tokio::test]
    async fn calls_records_each_question_set() {
        let judge = FakeJudge::failing(UnavailableReason::Network);
        let _ = judge.decide(json!({}), one_question()).await;
        assert_eq!(
            serde_json::to_value(judge.calls()).ok(),
            serde_json::to_value(vec![one_question()]).ok()
        );
    }
}
