//! What image and classifier models are asked and what they answer, in the shapes pi-ai gives
//! them, so a script or an extension reads the same JSON from micro as from pi.

use micro_models::ModelDef;
use micro_models::TokenUsage;
use serde::de::MapAccess;
use serde::de::SeqAccess;
use serde::de::Visitor;
use serde::ser::SerializeMap;
use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use serde::Serializer;
use serde_json::Value;

/// A JSON object whose keys keep the order they were written in, which is the order a choice
/// question lists its options and a result lists its answers.
#[derive(Debug, Clone, PartialEq)]
pub struct Ordered<V>(pub Vec<(String, V)>);

impl<V> Default for Ordered<V> {
    fn default() -> Self {
        Ordered(Vec::new())
    }
}

impl<V> Ordered<V> {
    pub fn get(&self, key: &str) -> Option<&V> {
        self.0
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value)
    }

    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(|(name, _)| name.as_str())
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &V)> {
        self.0.iter().map(|(name, value)| (name.as_str(), value))
    }
}

impl<V: Serialize> Serialize for Ordered<V> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (key, value) in &self.0 {
            map.serialize_entry(key, value)?;
        }
        map.end()
    }
}

impl<'de, V: Deserialize<'de>> Deserialize<'de> for Ordered<V> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct InOrder<V>(std::marker::PhantomData<V>);

        impl<'de, V: Deserialize<'de>> Visitor<'de> for InOrder<V> {
            type Value = Ordered<V>;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("an object, or a list of [key, value] pairs")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<Ordered<V>, A::Error> {
                let mut entries = Vec::new();
                while let Some((key, value)) = access.next_entry::<String, V>()? {
                    entries.retain(|(known, _): &(String, V)| *known != key);
                    entries.push((key, value));
                }
                Ok(Ordered(entries))
            }

            /// Pairs carry the order through anything that reads JSON objects into sorted maps on
            /// the way, as micro's own message passing does.
            fn visit_seq<A: SeqAccess<'de>>(self, mut access: A) -> Result<Ordered<V>, A::Error> {
                let mut entries = Vec::new();
                while let Some((key, value)) = access.next_element::<(String, V)>()? {
                    entries.retain(|(known, _): &(String, V)| *known != key);
                    entries.push((key, value));
                }
                Ok(Ordered(entries))
            }
        }

        deserializer.deserialize_any(InOrder(std::marker::PhantomData))
    }
}

/// Token counts of one request and what they cost at the model's catalog price.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PricedUsage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub total_tokens: u64,
    pub cost: UsageCost,
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageCost {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
    pub total: f64,
}

impl PricedUsage {
    /// Price these tokens at what `model` charges.
    pub fn priced(model: &ModelDef, tokens: TokenUsage) -> PricedUsage {
        let cost = model.price(tokens);
        PricedUsage {
            input: tokens.input,
            output: tokens.output,
            cache_read: tokens.cache_read,
            cache_write: tokens.cache_write,
            total_tokens: tokens.total(),
            cost: UsageCost {
                input: cost.input,
                output: cost.output,
                cache_read: cost.cache_read,
                cache_write: cost.cache_write,
                total: cost.total(),
            },
        }
    }

    pub fn tokens(&self) -> TokenUsage {
        TokenUsage::new(self.input, self.output).with_cache(self.cache_read, self.cache_write)
    }
}

/// How a request to an image or classifier model ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    #[default]
    Stop,
    Error,
    Aborted,
}

/// A block of text or a base64 image, going into an image model or coming out of one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ImagesContent {
    Text {
        text: String,
    },
    Image {
        /// Base64, without a `data:` prefix.
        data: String,
        #[serde(rename = "mimeType")]
        mime_type: String,
    },
}

/// What an image model is asked to draw from: a prompt, and optionally images to edit or follow.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ImagesContext {
    pub input: Vec<ImagesContent>,
}

/// What an image model answered.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantImages {
    pub api: String,
    pub provider: String,
    pub model: String,
    pub output: Vec<ImagesContent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<PricedUsage>,
    pub stop_reason: Outcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    /// Unix milliseconds.
    pub timestamp: i64,
}

impl AssistantImages {
    /// An answer with nothing in it yet, addressed from `model`.
    pub fn empty(model: &ModelDef) -> AssistantImages {
        AssistantImages {
            api: micro_models::wire_api_name(model.api).to_string(),
            provider: model.provider.clone(),
            model: model.id.clone(),
            output: Vec::new(),
            response_id: None,
            usage: None,
            stop_reason: Outcome::Stop,
            error_message: None,
            timestamp: now_ms(),
        }
    }

    /// The same answer, ended by `error`.
    pub fn failed(mut self, error: impl Into<String>) -> AssistantImages {
        self.stop_reason = Outcome::Error;
        self.error_message = Some(error.into());
        self
    }
}

/// What `Yes` and `No` mean for a yes-or-no question.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct BoolCriteria {
    #[serde(default, rename = "true")]
    pub yes: String,
    #[serde(default, rename = "false")]
    pub no: String,
}

/// One typed question about the state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ClassifierQuestion {
    /// Pick one of several options, each described by its criterion.
    Choice {
        instructions: String,
        criteria: Ordered<String>,
    },
    /// Rate on levels described in order, lowest first.
    Score {
        instructions: String,
        criteria: Vec<String>,
    },
    /// Answer yes or no.
    Bool {
        instructions: String,
        criteria: BoolCriteria,
    },
}

impl ClassifierQuestion {
    pub fn instructions(&self) -> &str {
        match self {
            ClassifierQuestion::Choice { instructions, .. }
            | ClassifierQuestion::Score { instructions, .. }
            | ClassifierQuestion::Bool { instructions, .. } => instructions,
        }
    }
}

/// The state to judge and the questions to answer about it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClassifierContext {
    /// A JSON object.
    pub state: Value,
    pub questions: Ordered<ClassifierQuestion>,
}

/// One answer, shaped by the question it answers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ClassifierAnswer {
    Choice {
        choice: String,
        probabilities: Ordered<f64>,
        confidence: f64,
    },
    Score {
        score: f64,
        confidence: f64,
    },
    Bool {
        /// How likely the answer is yes.
        probability: f64,
    },
}

/// What a classifier answered.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassifierResult {
    pub api: String,
    pub provider: String,
    pub model: String,
    pub answers: Ordered<ClassifierAnswer>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<PricedUsage>,
    pub stop_reason: Outcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    /// Unix milliseconds.
    pub timestamp: i64,
}

impl ClassifierResult {
    pub fn empty(model: &ModelDef) -> ClassifierResult {
        ClassifierResult {
            api: micro_models::wire_api_name(model.api).to_string(),
            provider: model.provider.clone(),
            model: model.id.clone(),
            answers: Ordered::default(),
            usage: None,
            stop_reason: Outcome::Stop,
            error_message: None,
            timestamp: now_ms(),
        }
    }

    pub fn failed(mut self, error: impl Into<String>) -> ClassifierResult {
        self.answers = Ordered::default();
        self.stop_reason = Outcome::Error;
        self.error_message = Some(error.into());
        self
    }
}

/// How a classification is carried out, beyond what is asked.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct ClassifyOptions {
    /// Divides the answer logits before they are normalized. Above 1 softens the distribution.
    /// Classifiers that cannot apply it ignore it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
}

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A choice question's options are labeled in the order they were written, so the order has to
    /// survive reading.
    #[test]
    fn options_keep_the_order_they_were_written_in() {
        let question: ClassifierQuestion = serde_json::from_str(
            r#"{"type": "choice", "instructions": "Which?",
                "criteria": {"zebra": "z", "apple": "a", "mango": "m"}}"#,
        )
        .unwrap();

        let as_pairs: ClassifierQuestion = serde_json::from_value(json!({
            "type": "choice",
            "instructions": "Which?",
            "criteria": [["zebra", "z"], ["apple", "a"], ["mango", "m"]],
        }))
        .unwrap();
        assert_eq!(as_pairs, question, "pairs read the same as an object");

        let ClassifierQuestion::Choice { criteria, .. } = &question else {
            panic!("a choice question");
        };
        assert_eq!(
            criteria.keys().collect::<Vec<_>>(),
            vec!["zebra", "apple", "mango"]
        );
        assert_eq!(
            serde_json::to_string(&question).unwrap(),
            r#"{"type":"choice","instructions":"Which?","criteria":{"zebra":"z","apple":"a","mango":"m"}}"#
        );
    }

    #[test]
    fn a_bool_question_reads_true_and_false() {
        let question: ClassifierQuestion = serde_json::from_value(json!({
            "type": "bool",
            "instructions": "Approved?",
            "criteria": { "true": "Approval", "false": "No approval" },
        }))
        .unwrap();
        assert_eq!(
            question,
            ClassifierQuestion::Bool {
                instructions: "Approved?".into(),
                criteria: BoolCriteria {
                    yes: "Approval".into(),
                    no: "No approval".into()
                },
            }
        );
    }

    #[test]
    fn an_image_block_names_its_mime_type_as_pi_does() {
        let block = ImagesContent::Image {
            data: "AAAA".into(),
            mime_type: "image/png".into(),
        };
        assert_eq!(
            serde_json::to_value(&block).unwrap(),
            json!({ "type": "image", "data": "AAAA", "mimeType": "image/png" })
        );
    }
}
