//! Optional LLM assistant. Credentials stay in local storage and are never
//! exported with the dive log.

use std::convert::Infallible;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use benthic_core::DiveLog;
use rig::operation::Completion;
use rig::prelude::*;
use rig::providers::{deepseek, gemini, groq, openrouter};
use rig::DynModel;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiProvider {
    #[default]
    OpenRouter,
    Groq,
    Gemini,
    DeepSeek,
}

impl AiProvider {
    pub const ALL: [AiProvider; 4] = [
        AiProvider::OpenRouter,
        AiProvider::Groq,
        AiProvider::Gemini,
        AiProvider::DeepSeek,
    ];

    pub fn label(self) -> &'static str {
        match self {
            AiProvider::OpenRouter => "OpenRouter",
            AiProvider::Groq => "Groq",
            AiProvider::Gemini => "Google Gemini",
            AiProvider::DeepSeek => "DeepSeek",
        }
    }

    pub fn default_model(self) -> &'static str {
        match self {
            AiProvider::OpenRouter => "openai/gpt-4o-mini",
            AiProvider::Groq => "llama-3.1-8b-instant",
            AiProvider::Gemini => "gemini-2.0-flash",
            AiProvider::DeepSeek => "deepseek-chat",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AiConfig {
    #[serde(default)]
    pub provider: AiProvider,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub api_key: String,
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            provider: AiProvider::default(),
            model: AiProvider::default().default_model().to_string(),
            api_key: String::new(),
        }
    }
}

pub fn load_config() -> AiConfig {
    crate::storage::load_ai()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn save_config(config: &AiConfig) -> Result<(), String> {
    let text = serde_json::to_string(config).map_err(|error| error.to_string())?;
    crate::storage::save_ai(&text)
}

fn model(config: &AiConfig) -> Result<DynModel<Completion>, String> {
    let key = config.api_key.trim();
    if key.is_empty() {
        return Err("Enter an API key first.".to_string());
    }
    let model = if config.model.trim().is_empty() {
        config.provider.default_model()
    } else {
        config.model.trim()
    };
    Ok(match config.provider {
        AiProvider::OpenRouter => openrouter::new(key).chat(model).erase(),
        AiProvider::Groq => groq::new(key).chat(model).erase(),
        AiProvider::Gemini => gemini::Gemini::new(key).completion(model).erase(),
        AiProvider::DeepSeek => deepseek::new(key).chat(model).erase(),
    })
}

#[derive(Debug, Deserialize)]
struct SearchArgs {
    query: String,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Serialize)]
struct DiveInfo {
    id: u32,
    number: i32,
    when: String,
    site: String,
    duration_seconds: Option<i32>,
    max_depth_m: Option<f64>,
    trip: Option<u32>,
}

struct SearchDives {
    log: DiveLog,
}

impl Tool for SearchDives {
    const NAME: &'static str = "search_dives";
    type Error = Infallible;
    type Args = SearchArgs;
    type Output = Vec<DiveInfo>;

    fn description(&self) -> String {
        "Search the dive log by dive number, internal id, site name, buddy, tags, notes, or date. Returns matching dives with their internal ids.".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Dive number, site, buddy, tag, date, or free-text search." },
                "limit": { "type": "integer", "minimum": 1, "maximum": 50 }
            },
            "required": ["query"],
            "additionalProperties": false
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let query = args.query.trim().to_lowercase();
        let limit = args.limit.unwrap_or(20).clamp(1, 50);
        let mut matches: Vec<DiveInfo> = self
            .log
            .dives
            .iter()
            .filter(|dive| {
                let site = self.log.site_name_of(dive).unwrap_or("");
                let text = format!(
                    "{} {} {} {} {} {}",
                    dive.id,
                    dive.number,
                    site,
                    dive.buddy,
                    dive.tags.join(" "),
                    dive.notes
                )
                .to_lowercase();
                text.contains(&query)
            })
            .map(|dive| DiveInfo {
                id: dive.id,
                number: dive.number,
                when: benthic_core::datetime_local(dive.when),
                site: self.log.site_name_of(dive).unwrap_or("").to_string(),
                duration_seconds: dive.duration().map(|duration| duration.seconds),
                max_depth_m: dive.max_depth().map(|depth| depth.meters()),
                trip: dive.trip_id,
            })
            .collect();
        matches.sort_by_key(|dive| std::cmp::Reverse(dive.when.clone()));
        matches.truncate(limit);
        Ok(matches)
    }
}

pub async fn run_prompt(config: &AiConfig, log: DiveLog, prompt: String) -> Result<String, String> {
    let agent = AgentBuilder::new(model(config)?)
        .preamble(
            "You are the assistant inside benthic, a dive log. Use search_dives whenever the \
             user refers to a dive by number, site, date, or description. Never invent a dive \
             id. Answer from tool results. This first version can inspect dives but cannot \
             modify them.",
        )
        .tool(SearchDives { log })
        .default_max_turns(5)
        .build();
    let response = agent
        .prompt(prompt)
        .max_turns(5)
        .await
        .map_err(|error| error.to_string())?;
    Ok(response.output())
}
