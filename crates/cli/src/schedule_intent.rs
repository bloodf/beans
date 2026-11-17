//! Notices when a user's message asks for something to happen regularly, wherever the ask sits
//! in the message and whatever language it is in: "summarize my inbox and, from now on, do it
//! every morning", "resume os emails e faça isso toda segunda às 9", "毎朝これをやって".
//!
//! Before a turn that the user's message starts in a direct chat, the small model of the bot's
//! provider (the one Auto-review uses) reads the message once. It answers whether the message
//! asks for a repeat, quotes the words that do, and writes the schedule in the one form
//! `crate::schedule` reads, from the same `REQUESTS` list the routines tool shows the bot. The
//! turn gets that as a note. The bot still sets the routine up itself with the routines tool,
//! so a wrong read costs the bot a moment of judgement, never a routine nobody asked for.

use std::sync::{Arc, LazyLock};
use std::time::Duration;

use beans_agent::provider::AssistantAccumulator;
use beans_agent::types::{LlmMessage, StopReason, UserMessage};
use beans_agent::{ModelRequest, Provider, RequestOptions, ThinkingLevel, Usage};
use futures::StreamExt;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

/// How long a turn waits for the read before it goes on without one.
pub const WAIT: Duration = Duration::from_secs(5);

/// The most of a message the read sees. A longer one is read from its start and its end, where
/// people usually put what they want done.
const MAX_MESSAGE_CHARS: usize = 12_000;

/// The longest quote and task a read may hand the turn.
const MAX_WORDS_CHARS: usize = 300;
const MAX_TASK_CHARS: usize = 500;

/// What the read found: the user asked for something to happen regularly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Intent {
    /// The user's words that ask for the repeat, as they wrote them.
    pub words: String,
    /// What to do each time, as the reader put it.
    pub task: Option<String>,
    /// The schedule as stored (`every 1d`, `0 9 * * 1`), when the reader wrote one that
    /// `crate::schedule::parse` accepts.
    pub schedule: Option<String>,
}

/// What one read returned, and what it cost.
pub struct Read {
    pub intent: Option<Intent>,
    pub usage: Usage,
}

static SYSTEM_PROMPT: LazyLock<String> = LazyLock::new(|| {
    let requests = crate::schedule::REQUESTS
        .iter()
        .map(|(request, schedule)| format!("{request}: {schedule}"))
        .collect::<Vec<_>>()
        .join("; ");
    format!(
        "You read one message a person sent to their assistant and say whether it asks the assistant to do something \
         again and again on a schedule: every day, each Monday, every two hours, on weekdays at 9, from now on every \
         morning. The message can be in any language, or several, and the ask can be anywhere in it: at the start, in the \
         middle of a sentence, or after other requests.\n\
         Only a request for the assistant to repeat something counts. These do not: something done once, even at a time \
         (\"remind me tomorrow\", \"send it at 5\"); a question about schedules; the person's own habits (\"I run every \
         morning\"); something quoted or described rather than asked for.\n\n\
         Answer with JSON only, nothing else. Either {{\"recurring\": false}} or\n\
         {{\"recurring\": true, \"words\": \"the words that ask for the repeat, copied exactly from the message\", \"task\": \
         \"what to do each time, in the message's language\", \"schedule\": \"the schedule\"}}\n\n\
         Write the schedule in this form, never in the person's words: every 30m, every 2h, every 1d, every 2w, or five \
         cron fields (minute hour day-of-month month day-of-week) in the person's local time, at most once per five \
         minutes. The same request is the same schedule in every language: {requests}. Without a time, daily is every 1d \
         and weekly every 1w. When you cannot tell the schedule, leave it \"\"."
    )
});

/// Reads `message` with `provider`, the small model of the bot's provider. Any failure reads as
/// nothing found, so the turn goes on as it would have.
pub async fn read(provider: &Arc<dyn Provider>, thinking: Option<ThinkingLevel>, chat_id: &str, message: &str, cancel: &CancellationToken) -> Read {
    let request = ModelRequest {
        system_prompt: SYSTEM_PROMPT.clone(),
        messages: vec![LlmMessage::User(UserMessage::text(clipped(message)))],
        tools: Vec::new(),
        cache_points: Vec::new(),
        // The answer is short; the rest is room for a model that reasons at its lowest effort.
        max_tokens: Some(2048),
        options: match thinking {
            Some(ThinkingLevel::Off) => RequestOptions::default().with_session_id(chat_id).with_hooks(Arc::new(crate::plugins::review::Steady)),
            _ => RequestOptions::default().with_session_id(chat_id),
        },
    };
    let mut stream = provider.stream(request, cancel.clone()).await;
    let mut acc = AssistantAccumulator::new(provider.provider_id(), provider.model_id());
    while let Some(event) = stream.next().await {
        acc.apply(&event);
    }
    let answer = acc.finish(cancel.is_cancelled());
    let intent = match answer.stop_reason {
        StopReason::Aborted | StopReason::Error => {
            tracing::debug!(error = ?answer.error_message, "reading a message for a schedule");
            None
        }
        _ => read_answer(&answer.text(), message),
    };
    Read { intent, usage: answer.usage }
}

/// The reader's JSON, tolerating prose or a code fence around it. A quote the message does not
/// contain is a reader's invention, and reads as nothing found.
pub fn read_answer(reply: &str, message: &str) -> Option<Intent> {
    let start = reply.find('{')?;
    let end = reply.rfind('}')?;
    let value: Value = serde_json::from_str(reply.get(start..=end)?).ok()?;
    let recurring = match &value["recurring"] {
        Value::Bool(recurring) => *recurring,
        Value::String(recurring) => recurring.eq_ignore_ascii_case("true"),
        _ => false,
    };
    if !recurring {
        return None;
    }
    let text = |key: &str| value[key].as_str().map(str::trim).filter(|text| !text.is_empty()).map(str::to_string);
    let words = text("words").filter(|words| words.chars().count() <= MAX_WORDS_CHARS && quotes(message, words))?;
    let task = text("task").map(|task| clip(&task, MAX_TASK_CHARS));
    let schedule = text("schedule").and_then(|schedule| crate::schedule::parse(&schedule).ok()).map(|schedule| schedule.canonical());
    Some(Intent { words, task, schedule })
}

impl Intent {
    /// The note the turn reads after the user's message. It is not stored, like the turn's other
    /// notes, and it leaves the decision to the bot.
    pub fn note(&self) -> String {
        let mut note = format!(
            "[Before this turn, a read of the user's latest message found it asks for something to happen regularly, in the \
             words \"{}\"",
            self.words
        );
        if let Some(task) = &self.task {
            note.push_str(&format!("; each time: {task}"));
        }
        match self.schedule.as_deref().and_then(|schedule| crate::schedule::parse_stored(schedule).ok()) {
            Some(schedule) => note.push_str(&format!("; as a schedule: {} ({})", schedule.canonical(), schedule.describe())),
            None => note.push_str("; write the schedule yourself"),
        }
        note.push_str(
            ". If they want that, set it up with the routines tool, unless one of your routines already does it, along with \
             anything else they asked for now, and tell them the schedule and its next run in their language. If they did not \
             mean it, ignore this note.]",
        );
        note
    }
}

/// The note for what a local model on the Runner flagged (`crate::schedule_models`): the clause
/// it read as a request to repeat something, quoted from the message. It is a small model's
/// guess, so the note says so and leaves the schedule to the bot.
pub fn local_note(words: &str) -> String {
    format!(
        "[Before this turn, a local check of the user's latest message found it may ask for something to happen regularly, \
         around the words \"{words}\". If it does, set it up with the routines tool and write the schedule yourself, unless \
         one of your routines already does it, along with anything else they asked for now, and tell them the schedule and its \
         next run in their language. If it does not, ignore this note.]"
    )
}

/// Whether `words` appear in `message`, ignoring case and how the spaces fall.
fn quotes(message: &str, words: &str) -> bool {
    let squeeze = |text: &str| text.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
    squeeze(message).contains(&squeeze(words))
}

/// A message the read can take: whole, or its start and end.
fn clipped(message: &str) -> String {
    let count = message.chars().count();
    if count <= MAX_MESSAGE_CHARS {
        return message.to_string();
    }
    let half = MAX_MESSAGE_CHARS / 2;
    let start: String = message.chars().take(half).collect();
    let end: String = message.chars().skip(count - half).collect();
    format!("{start}\n…\n{end}")
}

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut clipped: String = text.chars().take(max).collect();
    clipped.push('…');
    clipped
}

#[cfg(test)]
mod tests {
    use super::*;
    use beans_agent::provider::AssistantEvent;
    use beans_agent::AssistantEventStream;

    /// The reader's answers to each message in turn, and the requests it got.
    struct Reader {
        answers: std::sync::Mutex<Vec<&'static str>>,
        requests: std::sync::Mutex<Vec<ModelRequest>>,
    }

    impl Reader {
        fn answering(answers: &[&'static str]) -> Arc<Reader> {
            Arc::new(Reader {
                answers: std::sync::Mutex::new(answers.to_vec()),
                requests: Default::default(),
            })
        }
    }

    #[async_trait::async_trait]
    impl Provider for Reader {
        fn provider_id(&self) -> &str {
            "reader"
        }
        fn model_id(&self) -> &str {
            "small"
        }
        async fn stream(&self, request: ModelRequest, _cancel: CancellationToken) -> AssistantEventStream {
            self.requests.lock().unwrap().push(request);
            let answer = self.answers.lock().unwrap().remove(0);
            let events = if answer == "ERROR" {
                vec![AssistantEvent::Error {
                    message: "overloaded".into(),
                    aborted: false,
                }]
            } else {
                vec![
                    AssistantEvent::TextStart { index: 0 },
                    AssistantEvent::TextDelta { index: 0, delta: answer.into() },
                    AssistantEvent::TextEnd { index: 0 },
                    AssistantEvent::Done {
                        stop_reason: StopReason::Stop,
                        usage: Default::default(),
                    },
                ]
            };
            Box::pin(futures::stream::iter(std::iter::once(AssistantEvent::Start).chain(events)))
        }
    }

    /// The ask sits mid-sentence, in English and in Portuguese: each reads as the same schedule.
    #[tokio::test]
    async fn an_ask_in_the_middle_of_a_sentence_reads_the_same_in_any_language() {
        let english = "Can you summarize my unread email and, if it's not too much trouble, run this daily so I see it each morning?";
        let portuguese = "Pode resumir meus e-mails não lidos e agende para esse prompt rodar diariamente, por favor, obrigado!";
        let reader = Reader::answering(&[
            r#"{"recurring": true, "words": "run this daily", "task": "Summarize my unread email", "schedule": "every 1d"}"#,
            "```json\n{\"recurring\": true, \"words\": \"agende para esse prompt rodar diariamente\", \"task\": \"Resumir meus e-mails não lidos\", \"schedule\": \"every day\"}\n```",
        ]);
        let provider: Arc<dyn Provider> = reader.clone();
        let cancel = CancellationToken::new();
        let first = read(&provider, Some(ThinkingLevel::Off), "chat", english, &cancel).await.intent.unwrap();
        let second = read(&provider, Some(ThinkingLevel::Off), "chat", portuguese, &cancel).await.intent.unwrap();
        assert_eq!(first.schedule.as_deref(), Some("every 1d"));
        assert_eq!(second.schedule, first.schedule, "the same request, the same schedule");
        assert_eq!(second.words, "agende para esse prompt rodar diariamente");
        assert!(first.note().contains("as a schedule: every 1d (Every day)"));
        assert!(second.note().contains("in the words \"agende para esse prompt rodar diariamente\""));

        // The reader saw the whole message, and the one form to write.
        let requests = reader.requests.lock().unwrap();
        assert!(requests[0].system_prompt.contains("in the middle of a sentence"));
        assert!(requests[0].system_prompt.contains("daily, with no time of day: every 1d"));
        assert!(matches!(&requests[1].messages[0], LlmMessage::User(user) if format!("{:?}", user.content).contains("rodar diariamente")));
    }

    #[tokio::test]
    async fn nothing_found_and_a_failed_read_leave_the_turn_as_it_was() {
        let reader = Reader::answering(&[r#"{"recurring": false}"#, "ERROR", "I think they want it daily."]);
        let provider: Arc<dyn Provider> = reader.clone();
        let cancel = CancellationToken::new();
        for message in ["I go running every morning.", "Run this daily", "Run this daily"] {
            assert_eq!(read(&provider, None, "chat", message, &cancel).await.intent, None, "{message}");
        }
    }

    #[test]
    fn answers_are_held_to_the_message() {
        let message = "Resume os emails e, a partir de agora, faça isso toda segunda às 9, ok?";
        let answer = |json: &str| read_answer(json, message);
        // A schedule the parser takes is stored as the parser stores it.
        let found = answer(r#"{"recurring": true, "words": "toda segunda às 9", "task": "Resumir os emails", "schedule": "0 9 * * mon"}"#).unwrap();
        assert_eq!(
            found,
            Intent {
                words: "toda segunda às 9".into(),
                task: Some("Resumir os emails".into()),
                schedule: Some("0 9 * * mon".into())
            }
        );
        assert!(found.note().contains("as a schedule: 0 9 * * mon (Every Monday at 9:00 AM)"));
        // Case and spacing may differ from the message; the words may not.
        assert!(answer(r#"{"recurring": true, "words": "TODA  segunda às 9", "schedule": "0 9 * * 1"}"#).is_some());
        assert_eq!(answer(r#"{"recurring": true, "words": "every Monday at 9", "schedule": "0 9 * * 1"}"#), None, "not the user's words");
        assert_eq!(answer(r#"{"recurring": true, "words": "", "schedule": "0 9 * * 1"}"#), None);
        // A schedule the parser refuses, or the user's words as a schedule, leaves the schedule to the bot.
        let unsure = answer(r#"{"recurring": "true", "words": "toda segunda às 9", "schedule": "toda segunda"}"#).unwrap();
        assert_eq!(unsure.schedule, None);
        assert!(unsure.note().contains("write the schedule yourself"));
        assert_eq!(answer(r#"{"recurring": true, "words": "toda segunda às 9", "schedule": "* * * * *"}"#).unwrap().schedule, None);
        // Off-format answers find nothing.
        for reply in ["", "yes", "{not json}", r#"{"recurring": false, "words": "toda segunda às 9"}"#, r#"{"recurring": 1}"#] {
            assert_eq!(answer(reply), None, "{reply}");
        }
    }

    #[test]
    fn a_long_message_is_read_from_both_ends() {
        let long = format!("{}run this every day{}", "a".repeat(20_000), "b".repeat(100));
        let read = clipped(&long);
        assert!(read.chars().count() < MAX_MESSAGE_CHARS + 10);
        assert!(read.contains("run this every day"));
        assert_eq!(clipped("short"), "short");
        assert_eq!(clip(&"é".repeat(600), MAX_TASK_CHARS).chars().count(), MAX_TASK_CHARS + 1);
    }
}
