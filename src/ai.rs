//! Optional LLM assistant. Credentials and threads stay in local storage and
//! are never exported with the dive log or synced.
//!
//! The model only sees the log through read-only tools that run locally, so
//! it answers from real data instead of a pasted dump of the whole log.

use std::convert::Infallible;

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use benthic_core::units::Timestamp;
use benthic_core::{Dive, DiveLog, Preferences};
use rig::agent::MultiTurnStreamItem;
use rig::completion::Message;
use rig::operation::Completion;
use rig::prelude::*;
use rig::providers::openai::wire::{OpenAIConfig, OPENROUTER};
use rig::providers::{deepseek, gemini, groq, openrouter};
use rig::streaming::{Item, StreamEvent};
use rig::DynModel;

/// How many past threads are kept on the device.
const MAX_THREADS: usize = 20;
/// Model calls allowed per answer (tool round-trips included).
const MAX_TURNS: usize = 8;

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
    /// Development override for OpenRouter's endpoint (an OpenAI-compatible
    /// base URL), used to test against a local mock. Not shown in the UI.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub base_url: String,
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            provider: AiProvider::default(),
            model: AiProvider::default().default_model().to_string(),
            api_key: String::new(),
            base_url: String::new(),
        }
    }
}

impl AiConfig {
    pub fn is_ready(&self) -> bool {
        !self.api_key.trim().is_empty()
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
        return Err("No API key configured.".to_string());
    }
    let model = if config.model.trim().is_empty() {
        config.provider.default_model()
    } else {
        config.model.trim()
    };
    Ok(match config.provider {
        AiProvider::OpenRouter if !config.base_url.trim().is_empty() => {
            OpenAIConfig::with_key(&OPENROUTER, key)
                .with_base_url(config.base_url.trim())
                .client()
                .chat(model)
                .erase()
        }
        AiProvider::OpenRouter => openrouter::new(key).chat(model).erase(),
        AiProvider::Groq => groq::new(key).chat(model).erase(),
        AiProvider::Gemini => gemini::Gemini::new(key).completion(model).erase(),
        AiProvider::DeepSeek => deepseek::new(key).chat(model).erase(),
    })
}

// ---------------------------------------------------------------------------
// Threads
// ---------------------------------------------------------------------------

/// One displayed exchange step.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Entry {
    User {
        text: String,
    },
    Assistant {
        text: String,
    },
    /// A tool the assistant ran, with a short human summary of the call.
    Tool {
        name: String,
        summary: String,
    },
    Error {
        text: String,
    },
}

/// A conversation: what the UI shows plus the provider-facing history that
/// is replayed on the next question.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Thread {
    pub id: u64,
    pub updated: Timestamp,
    #[serde(default)]
    pub entries: Vec<Entry>,
    #[serde(default)]
    pub history: Vec<Message>,
}

impl Thread {
    pub fn new(now: Timestamp) -> Self {
        Self {
            id: now.max(0) as u64,
            updated: now,
            ..Default::default()
        }
    }

    /// The first question, used as the thread's title.
    pub fn title(&self) -> Option<&str> {
        self.entries.iter().find_map(|entry| match entry {
            Entry::User { text } => Some(text.as_str()),
            _ => None,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

pub fn load_threads() -> Vec<Thread> {
    crate::storage::load_ai_threads()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// Store `thread` (most recent first), keeping at most [`MAX_THREADS`].
pub fn save_thread(thread: &Thread) {
    if thread.is_empty() {
        return;
    }
    let mut threads = load_threads();
    threads.retain(|other| other.id != thread.id);
    threads.insert(0, thread.clone());
    threads.truncate(MAX_THREADS);
    write_threads(&threads);
}

pub fn delete_thread(id: u64) {
    let mut threads = load_threads();
    threads.retain(|thread| thread.id != id);
    write_threads(&threads);
}

fn write_threads(threads: &[Thread]) {
    if let Ok(text) = serde_json::to_string(threads) {
        let _ = crate::storage::save_ai_threads(&text);
    }
}

// ---------------------------------------------------------------------------
// Running a question
// ---------------------------------------------------------------------------

/// What the assistant knows about the app's state when a question is asked.
#[derive(Clone)]
pub struct Context {
    pub log: DiveLog,
    pub prefs: Preferences,
    /// The dive open in the detail pane, if any.
    pub selected: Option<u32>,
}

/// Progress reported while an answer streams in.
pub enum Event {
    Text(String),
    Tool { name: String, summary: String },
}

fn preamble(context: &Context) -> String {
    let language = match context.prefs.language {
        benthic_core::Language::French => "French",
        _ => "English",
    };
    let units = match context.prefs.units {
        benthic_core::UnitSystem::Metric => "metric (m, °C, bar, L)",
        benthic_core::UnitSystem::Imperial => "imperial (ft, °F, psi, cuft)",
    };
    let selected = context
        .selected
        .and_then(|id| context.log.dive_by_id(id))
        .map(|dive| {
            format!(
                "The user is currently looking at dive id {} ({}).",
                dive.id,
                dive_label(&context.log, dive)
            )
        })
        .unwrap_or_else(|| "No dive is currently open.".to_string());
    format!(
        "You are the assistant inside benthic, a scuba and freediving log. \
         Answer in {language}, concisely, using {units} units. \
         The log has {count} dives. {selected} \
         Use the tools to look things up: search_dives to find dives, get_dive for one \
         dive's details, log_stats for totals and records, list_sites for places. \
         Never invent dives, numbers or ids: only state what tools returned. \
         When you mention a specific dive, write it as [dive #NUMBER](dive:ID) so the user \
         can open it. You can read the log but not change it.",
        count = context.log.dives.len(),
    )
}

/// Ask `prompt` in `thread`, streaming progress through `on_event`. On success
/// the thread's history includes the new exchange.
pub async fn ask(
    config: &AiConfig,
    context: Context,
    thread: &mut Thread,
    prompt: String,
    mut on_event: impl FnMut(Event),
) -> Result<String, String> {
    let preamble = preamble(&context);
    let tr = crate::i18n::strings(context.prefs.language);
    let log = std::sync::Arc::new(context.log);
    let prefs = context.prefs;
    let agent = AgentBuilder::new(model(config)?)
        .preamble(preamble.as_str())
        .tool(SearchDives {
            log: log.clone(),
            prefs,
        })
        .tool(GetDive {
            log: log.clone(),
            prefs,
        })
        .tool(LogStats {
            log: log.clone(),
            prefs,
        })
        .tool(ListSites { log })
        .default_max_turns(MAX_TURNS)
        .build();

    let mut stream = agent
        .prompt(prompt.as_str())
        .history(thread.history.iter().cloned())
        .max_turns(MAX_TURNS)
        .stream();
    let mut text = String::new();
    while let Some(item) = stream.next().await {
        match item.map_err(|error| friendly_error(&error.to_string(), tr))? {
            MultiTurnStreamItem::StreamAssistantItem(Item::Event(StreamEvent::Text {
                text: delta,
                ..
            })) => {
                text.push_str(&delta);
                on_event(Event::Text(delta));
            }
            MultiTurnStreamItem::ToolCall { tool_call } => {
                let name = tool_call.function.name.to_string();
                let summary = summarize_call(&name, &tool_call.function.arguments);
                // Text streamed before a tool call is a preamble to it, not
                // the answer.
                text.clear();
                on_event(Event::Tool { name, summary });
            }
            MultiTurnStreamItem::ModelTurnRetried { .. } => text.clear(),
            MultiTurnStreamItem::FinalResponse(response) => {
                thread.history.extend(response.messages.iter().cloned());
                let output = response.output();
                if !output.trim().is_empty() {
                    text = output;
                }
            }
            _ => {}
        }
    }
    Ok(text)
}

/// Turn transport errors into something a diver can act on, keeping the
/// provider's own message when there is one.
fn friendly_error(error: &str, tr: &crate::i18n::Strings) -> String {
    let lower = error.to_lowercase();
    if lower.contains("401") || lower.contains("unauthorized") || lower.contains("invalid api key")
    {
        tr.ai_error_key.to_string()
    } else if lower.contains("429") || lower.contains("rate limit") {
        tr.ai_error_rate.to_string()
    } else if lower.contains("error sending request") || lower.contains("failed to fetch") {
        tr.ai_error_network.to_string()
    } else {
        error.to_string()
    }
}

fn summarize_call(name: &str, arguments: &serde_json::Map<String, Value>) -> String {
    let arg = |key: &str| arguments.get(key).and_then(Value::as_str).unwrap_or("");
    match name {
        SearchDives::NAME => {
            let mut parts: Vec<String> = Vec::new();
            for key in ["query", "site", "buddy", "from", "to"] {
                if !arg(key).is_empty() {
                    parts.push(arg(key).to_string());
                }
            }
            if let Some(depth) = arguments.get("min_depth_m").and_then(Value::as_f64) {
                parts.push(format!("≥ {depth} m"));
            }
            parts.join(" · ")
        }
        GetDive::NAME => arguments
            .get("number")
            .or_else(|| arguments.get("id"))
            .map(|value| format!("#{value}"))
            .unwrap_or_default(),
        _ => String::new(),
    }
}

// ---------------------------------------------------------------------------
// Tools
// ---------------------------------------------------------------------------

fn dive_label(log: &DiveLog, dive: &Dive) -> String {
    let site = log.site_name_of(dive).unwrap_or("unknown site");
    format!(
        "#{} at {} on {}",
        dive.number,
        site,
        benthic_core::datetime_local(dive.when)
    )
}

#[derive(Debug, Serialize)]
struct DiveSummary {
    id: u32,
    number: i32,
    when: String,
    site: String,
    duration: Option<String>,
    max_depth: Option<String>,
    trip: Option<String>,
    freedive: bool,
}

fn summarize(log: &DiveLog, prefs: &Preferences, dive: &Dive) -> DiveSummary {
    DiveSummary {
        id: dive.id,
        number: dive.number,
        when: benthic_core::datetime_local(dive.when),
        site: log.site_name_of(dive).unwrap_or("").to_string(),
        duration: dive.duration().map(benthic_core::units::format_duration),
        max_depth: dive.max_depth().map(|depth| prefs.depth(depth)),
        trip: dive
            .trip_id
            .and_then(|id| log.trip_by_id(id))
            .map(|trip| trip.location.clone()),
        freedive: dive.is_freedive(),
    }
}

/// Parse `YYYY-MM-DD` (or `YYYY-MM`, `YYYY`) as the start of that period.
fn parse_date(text: &str) -> Option<Timestamp> {
    let mut parts = text.trim().split('-');
    let year: i32 = parts.next()?.parse().ok()?;
    let month: u8 = parts.next().map_or(Some(1), |m| m.parse().ok())?;
    let day: u8 = parts.next().map_or(Some(1), |d| d.parse().ok())?;
    let month = time::Month::try_from(month).ok()?;
    let date = time::Date::from_calendar_date(year, month, day).ok()?;
    Some(date.midnight().assume_utc().unix_timestamp())
}

/// End of the period `text` names (the next day, month or year).
fn parse_date_end(text: &str) -> Option<Timestamp> {
    let start = parse_date(text)?;
    let date = time::OffsetDateTime::from_unix_timestamp(start)
        .ok()?
        .date();
    let next = match text.trim().split('-').count() {
        1 => date.replace_year(date.year() + 1).ok()?,
        2 => {
            let (year, month) = match date.month() {
                time::Month::December => (date.year() + 1, time::Month::January),
                month => (date.year(), month.next()),
            };
            time::Date::from_calendar_date(year, month, 1).ok()?
        }
        _ => date.next_day()?,
    };
    Some(next.midnight().assume_utc().unix_timestamp())
}

#[derive(Debug, Default, Deserialize)]
struct SearchArgs {
    #[serde(default)]
    query: String,
    #[serde(default)]
    site: String,
    #[serde(default)]
    buddy: String,
    #[serde(default)]
    from: String,
    #[serde(default)]
    to: String,
    #[serde(default)]
    min_depth_m: Option<f64>,
    #[serde(default)]
    freedive: Option<bool>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Serialize)]
struct SearchResult {
    total_matches: usize,
    dives: Vec<DiveSummary>,
}

struct SearchDives {
    log: std::sync::Arc<DiveLog>,
    prefs: Preferences,
}

impl SearchDives {
    fn search(&self, args: SearchArgs) -> SearchResult {
        let log = &*self.log;
        let query = args.query.trim().to_lowercase();
        let site = args.site.trim().to_lowercase();
        let buddy = args.buddy.trim().to_lowercase();
        let from = parse_date(&args.from);
        let to = parse_date_end(&args.to);
        let mut matches: Vec<&Dive> = log
            .dives
            .iter()
            .filter(|dive| {
                let site_name = log.site_name_of(dive).unwrap_or("").to_lowercase();
                (site.is_empty() || site_name.contains(&site))
                    && (buddy.is_empty()
                        || dive
                            .people_names()
                            .any(|name| name.to_lowercase().contains(&buddy)))
                    && from.is_none_or(|from| dive.when >= from)
                    && to.is_none_or(|to| dive.when < to)
                    && args.min_depth_m.is_none_or(|min| {
                        dive.max_depth().is_some_and(|depth| depth.meters() >= min)
                    })
                    && args
                        .freedive
                        .is_none_or(|wanted| dive.is_freedive() == wanted)
                    && (query.is_empty() || {
                        let trip = dive
                            .trip_id
                            .and_then(|id| log.trip_by_id(id))
                            .map(|trip| trip.location.as_str())
                            .unwrap_or("");
                        let text = format!(
                            "{} {} {} {} {} {} {} {}",
                            dive.number,
                            site_name,
                            dive.buddy,
                            dive.diveguide,
                            dive.suit,
                            dive.tags.join(" "),
                            dive.notes,
                            trip
                        )
                        .to_lowercase();
                        query
                            .split_whitespace()
                            .all(|word| text.contains(word.trim_start_matches('#')))
                    })
            })
            .collect();
        matches.sort_by_key(|dive| std::cmp::Reverse(dive.when));
        let total_matches = matches.len();
        let limit = args.limit.unwrap_or(20).clamp(1, 50);
        SearchResult {
            total_matches,
            dives: matches
                .into_iter()
                .take(limit)
                .map(|dive| summarize(log, &self.prefs, dive))
                .collect(),
        }
    }
}

impl Tool for SearchDives {
    const NAME: &'static str = "search_dives";
    type Error = Infallible;
    type Args = SearchArgs;
    type Output = SearchResult;

    fn description(&self) -> String {
        "Find dives, newest first. All filters are optional and combined. `query` matches \
         words in the number, site, people, suit, tags, notes and trip."
            .to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Free-text words, e.g. 'night wreck' or '82'." },
                "site": { "type": "string", "description": "Part of a site name." },
                "buddy": { "type": "string", "description": "Part of a buddy or guide name." },
                "from": { "type": "string", "description": "Start date: YYYY, YYYY-MM or YYYY-MM-DD." },
                "to": { "type": "string", "description": "End date (inclusive): YYYY, YYYY-MM or YYYY-MM-DD." },
                "min_depth_m": { "type": "number", "description": "Minimum maximum depth in metres." },
                "freedive": { "type": "boolean", "description": "Only freedives (true) or only scuba (false)." },
                "limit": { "type": "integer", "minimum": 1, "maximum": 50 }
            },
            "additionalProperties": false
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        Ok(self.search(args))
    }
}

#[derive(Debug, Deserialize)]
struct GetDiveArgs {
    #[serde(default)]
    id: Option<u32>,
    #[serde(default)]
    number: Option<i32>,
}

#[derive(Debug, Serialize)]
struct CylinderInfo {
    description: String,
    gas: String,
    size: Option<String>,
    start: Option<String>,
    end: Option<String>,
}

#[derive(Debug, Serialize)]
struct DiveDetails {
    #[serde(flatten)]
    summary: DiveSummary,
    mode: String,
    average_depth: Option<String>,
    water_temperature: Option<String>,
    air_temperature: Option<String>,
    buddy: String,
    guide: String,
    suit: String,
    rating: u8,
    tags: Vec<String>,
    notes: String,
    cylinders: Vec<CylinderInfo>,
    gear: Vec<String>,
    rmv_l_per_min: Option<f64>,
    computer: Option<String>,
    gps: Option<String>,
}

struct GetDive {
    log: std::sync::Arc<DiveLog>,
    prefs: Preferences,
}

impl GetDive {
    fn details(&self, args: GetDiveArgs) -> Result<DiveDetails, String> {
        let log = &*self.log;
        let prefs = &self.prefs;
        let dive = match (args.id, args.number) {
            (Some(id), _) => log.dive_by_id(id),
            (None, Some(number)) => log.dives.iter().find(|dive| dive.number == number),
            (None, None) => None,
        }
        .ok_or("No such dive. Use search_dives to find its id.")?;
        let computer = dive.primary_computer();
        Ok(DiveDetails {
            summary: summarize(log, prefs, dive),
            mode: format!("{:?}", computer.map(|dc| dc.divemode).unwrap_or_default()),
            average_depth: dive.average_depth().map(|depth| prefs.depth(depth)),
            water_temperature: dive
                .water_temp
                .or_else(|| computer.and_then(|dc| dc.water_temp))
                .map(|t| prefs.temperature(t)),
            air_temperature: dive
                .air_temp
                .or_else(|| computer.and_then(|dc| dc.air_temp))
                .map(|t| prefs.temperature(t)),
            buddy: dive.buddy.clone(),
            guide: dive.diveguide.clone(),
            suit: dive.suit.clone(),
            rating: dive.rating,
            tags: dive.tags.clone(),
            notes: dive.notes.clone(),
            cylinders: dive
                .cylinders
                .iter()
                .map(|cylinder| CylinderInfo {
                    description: cylinder.description.clone(),
                    gas: cylinder.gas.name(),
                    size: cylinder.size.map(|size| format!("{:.1} L", size.liters())),
                    start: cylinder.start_pressure.map(|p| prefs.pressure(p)),
                    end: cylinder.end_pressure.map(|p| prefs.pressure(p)),
                })
                .collect(),
            gear: dive
                .gear_ids
                .iter()
                .filter_map(|id| log.gear_by_id(*id))
                .map(|item| item.name.clone())
                .collect(),
            rmv_l_per_min: dive.rmv_l_per_min().map(|rmv| (rmv * 10.0).round() / 10.0),
            computer: computer
                .map(|dc| dc.model.clone())
                .filter(|model| !model.is_empty()),
            gps: log
                .dive_location(dive)
                .map(|l| format!("{:.5}, {:.5}", l.lat, l.lon)),
        })
    }
}

impl Tool for GetDive {
    const NAME: &'static str = "get_dive";
    type Error = Infallible;
    type Args = GetDiveArgs;
    type Output = Value;

    fn description(&self) -> String {
        "Full details of one dive: gases, pressures, temperatures, people, gear, notes. \
         Give either its internal `id` (from search results) or its logbook `number`."
            .to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": { "type": "integer", "description": "Internal dive id." },
                "number": { "type": "integer", "description": "Logbook dive number." }
            },
            "additionalProperties": false
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        Ok(match self.details(args) {
            Ok(details) => serde_json::to_value(details).unwrap_or(Value::Null),
            Err(error) => json!({ "error": error }),
        })
    }
}

#[derive(Debug, Default, Deserialize)]
struct StatsArgs {
    #[serde(default)]
    from: String,
    #[serde(default)]
    to: String,
}

#[derive(Debug, Serialize)]
struct Stats {
    dives: usize,
    freedives: usize,
    total_time: String,
    first_dive: Option<String>,
    last_dive: Option<String>,
    deepest: Option<DiveSummary>,
    longest: Option<DiveSummary>,
    coldest_water: Option<String>,
    sites: usize,
    dives_per_year: std::collections::BTreeMap<i32, usize>,
    top_sites: Vec<(String, usize)>,
    top_buddies: Vec<(String, usize)>,
}

struct LogStats {
    log: std::sync::Arc<DiveLog>,
    prefs: Preferences,
}

impl LogStats {
    fn stats(&self, args: StatsArgs) -> Stats {
        let log = &*self.log;
        let from = parse_date(&args.from);
        let to = parse_date_end(&args.to);
        let dives: Vec<&Dive> = log
            .dives
            .iter()
            .filter(|dive| from.is_none_or(|from| dive.when >= from))
            .filter(|dive| to.is_none_or(|to| dive.when < to))
            .collect();
        let total_seconds: i64 = dives
            .iter()
            .filter_map(|dive| dive.duration())
            .map(|d| d.seconds as i64)
            .sum();
        let mut per_year = std::collections::BTreeMap::new();
        let mut sites = std::collections::BTreeMap::<String, usize>::new();
        let mut buddies = std::collections::BTreeMap::<String, usize>::new();
        for dive in &dives {
            if let Ok(when) = time::OffsetDateTime::from_unix_timestamp(dive.when) {
                *per_year.entry(when.year()).or_insert(0) += 1;
            }
            if let Some(site) = log.site_name_of(dive) {
                *sites.entry(site.to_string()).or_insert(0) += 1;
            }
            for name in dive.people_names() {
                *buddies.entry(name.to_string()).or_insert(0) += 1;
            }
        }
        let top = |counts: std::collections::BTreeMap<String, usize>| {
            let mut list: Vec<(String, usize)> = counts.into_iter().collect();
            list.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            list.truncate(5);
            list
        };
        Stats {
            dives: dives.len(),
            freedives: dives.iter().filter(|dive| dive.is_freedive()).count(),
            total_time: format!(
                "{}h{:02}",
                total_seconds / 3600,
                (total_seconds % 3600) / 60
            ),
            first_dive: dives
                .iter()
                .min_by_key(|dive| dive.when)
                .map(|dive| benthic_core::datetime_local(dive.when)),
            last_dive: dives
                .iter()
                .max_by_key(|dive| dive.when)
                .map(|dive| benthic_core::datetime_local(dive.when)),
            deepest: dives
                .iter()
                .filter(|dive| !dive.is_freedive())
                .max_by_key(|dive| dive.max_depth().map(|d| d.mm))
                .map(|dive| summarize(log, &self.prefs, dive)),
            longest: dives
                .iter()
                .max_by_key(|dive| dive.duration().map(|d| d.seconds))
                .map(|dive| summarize(log, &self.prefs, dive)),
            coldest_water: dives
                .iter()
                .filter_map(|dive| dive.water_temp)
                .min_by_key(|t| t.mkelvin)
                .map(|t| self.prefs.temperature(t)),
            sites: sites.len(),
            dives_per_year: per_year,
            top_sites: top(sites),
            top_buddies: top(buddies),
        }
    }
}

impl Tool for LogStats {
    const NAME: &'static str = "log_stats";
    type Error = Infallible;
    type Args = StatsArgs;
    type Output = Stats;

    fn description(&self) -> String {
        "Totals and records for the whole log or a date range: dive count, total time, \
         deepest and longest dives, dives per year, most visited sites and buddies."
            .to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "from": { "type": "string", "description": "Start date: YYYY, YYYY-MM or YYYY-MM-DD." },
                "to": { "type": "string", "description": "End date (inclusive): YYYY, YYYY-MM or YYYY-MM-DD." }
            },
            "additionalProperties": false
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        Ok(self.stats(args))
    }
}

#[derive(Debug, Serialize)]
struct SiteInfo {
    name: String,
    dives: usize,
    country: Option<String>,
    gps: Option<String>,
}

struct ListSites {
    log: std::sync::Arc<DiveLog>,
}

impl Tool for ListSites {
    const NAME: &'static str = "list_sites";
    type Error = Infallible;
    type Args = Value;
    type Output = Vec<SiteInfo>;

    fn description(&self) -> String {
        "Every dive site with its number of dives, country and position, most dived first."
            .to_string()
    }

    fn parameters(&self) -> Value {
        json!({ "type": "object", "properties": {}, "additionalProperties": false })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        _args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let log = &*self.log;
        let counts = log.site_dive_counts();
        let mut sites: Vec<SiteInfo> = log
            .sites
            .iter()
            .map(|site| SiteInfo {
                name: site.name.clone(),
                dives: counts.get(&site.uuid).copied().unwrap_or(0),
                country: site.country.clone(),
                gps: log
                    .site_location(site.uuid)
                    .map(|l| format!("{:.5}, {:.5}", l.lat, l.lon)),
            })
            .collect();
        sites.sort_by(|a, b| b.dives.cmp(&a.dives).then_with(|| a.name.cmp(&b.name)));
        Ok(sites)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn demo() -> (std::sync::Arc<DiveLog>, Preferences) {
        let mut log = benthic_core::io::parse_auto(crate::actions::EXAMPLE_LOG).unwrap();
        log.prepare();
        (std::sync::Arc::new(log), Preferences::default())
    }

    #[test]
    fn search_combines_filters() {
        let (log, prefs) = demo();
        let tool = SearchDives {
            log: log.clone(),
            prefs,
        };
        let all = tool.search(SearchArgs::default());
        assert_eq!(all.total_matches, log.dives.len());

        let deep = tool.search(SearchArgs {
            min_depth_m: Some(40.0),
            ..Default::default()
        });
        assert!(deep.total_matches > 0);
        assert!(deep.dives.iter().all(|dive| !dive.freedive));

        let in_2024 = tool.search(SearchArgs {
            from: "2024".into(),
            to: "2024".into(),
            ..Default::default()
        });
        assert!(in_2024
            .dives
            .iter()
            .all(|dive| dive.when.starts_with("2024")));

        let by_site = tool.search(SearchArgs {
            site: "donator".into(),
            ..Default::default()
        });
        assert!(by_site.total_matches >= 1);
        assert!(by_site.dives.iter().all(|dive| dive.site == "Le Donator"));
    }

    #[test]
    fn get_dive_by_number_and_stats_cover_the_log() {
        let (log, prefs) = demo();
        let number = log.dives[0].number;
        let details = GetDive {
            log: log.clone(),
            prefs,
        }
        .details(GetDiveArgs {
            id: None,
            number: Some(number),
        })
        .unwrap();
        assert_eq!(details.summary.number, number);
        assert!(GetDive {
            log: log.clone(),
            prefs,
        }
        .details(GetDiveArgs {
            id: Some(99_999),
            number: None,
        })
        .is_err());

        let stats = LogStats {
            log: log.clone(),
            prefs,
        }
        .stats(StatsArgs::default());
        assert_eq!(stats.dives, log.dives.len());
        assert_eq!(
            stats.dives_per_year.values().sum::<usize>(),
            log.dives.len()
        );
        assert!(stats.deepest.is_some());
    }

    #[test]
    fn date_ranges_cover_whole_periods() {
        let start = parse_date("2024-02").unwrap();
        let end = parse_date_end("2024-02").unwrap();
        assert_eq!(end - start, 29 * 86_400);
        assert_eq!(
            parse_date_end("2024").unwrap() - parse_date("2024").unwrap(),
            366 * 86_400
        );
        assert!(parse_date("not a date").is_none());
    }

    #[test]
    fn threads_title_with_the_first_question() {
        let mut thread = Thread::new(10);
        assert!(thread.title().is_none());
        thread.entries.push(Entry::User {
            text: "Deepest dive?".into(),
        });
        thread.entries.push(Entry::Assistant { text: "#4".into() });
        assert_eq!(thread.title(), Some("Deepest dive?"));
        let text = serde_json::to_string(&thread).unwrap();
        assert_eq!(serde_json::from_str::<Thread>(&text).unwrap(), thread);
    }
}
