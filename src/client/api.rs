use std::{
    collections::HashMap,
    io::Read,
    sync::{Arc, Mutex},
    time::Duration,
};

use chrono::{DateTime, Utc};
use compact_str::CompactString;
use flate2::read::{DeflateDecoder, GzDecoder, ZlibDecoder};
use reqwest::{
    Client, StatusCode,
    header::{
        ACCEPT_ENCODING, CONTENT_ENCODING, ETAG, HeaderMap, HeaderValue, IF_MODIFIED_SINCE,
        IF_NONE_MATCH, LAST_MODIFIED,
    },
};
use serde_json::{Value, json};

use crate::{
    client::{config::ClientConfig, error::ClientError},
    domain::{
        BreadcrumbEntry, EventTag, ExceptionValue, IssueLevel, IssuePriority, IssueStatus,
        RequestEntry, SentryEventDetail, SentryIssue, SentryProject, StackFrame, UserContext,
    },
    id::{EventId, IssueId, ProjectId},
};

const MAX_ETAG_CACHE_ENTRIES: usize = 512;

#[derive(Debug, Clone, Default)]
struct ConditionalValidators {
    etag: Option<CompactString>,
    last_modified: Option<CompactString>,
}

#[derive(Debug, Clone)]
pub struct SentryApiClient {
    http: Client,
    config: ClientConfig,
    etag_cache: Arc<Mutex<HashMap<CompactString, ConditionalValidators>>>,
}

impl SentryApiClient {
    pub fn new(config: ClientConfig) -> Result<Self, ClientError> {
        let mut default_headers = HeaderMap::new();
        default_headers.insert(ACCEPT_ENCODING, HeaderValue::from_static("gzip, deflate"));

        let http = Client::builder()
            .default_headers(default_headers)
            .timeout(Duration::from_secs(20))
            .connect_timeout(Duration::from_secs(10))
            .tcp_keepalive(Duration::from_secs(30))
            .pool_idle_timeout(Duration::from_secs(90))
            .pool_max_idle_per_host(8)
            .user_agent("slim-tui/0.1.1")
            .build()
            .map_err(|e| ClientError::Http(e.to_string().into()))?;
        Ok(Self {
            http,
            config,
            etag_cache: Arc::new(Mutex::new(HashMap::with_capacity(64))),
        })
    }

    pub fn update_config(&mut self, config: ClientConfig) {
        let org_or_projects_changed = self.config.org != config.org
            || self.config.base_url != config.base_url
            || self.config.configured_projects != config.configured_projects
            || self.config.max_issues_per_poll != config.max_issues_per_poll;
        self.config = config;
        if org_or_projects_changed {
            self.clear_etag_cache();
        }
    }

    pub fn clear_etag_cache(&self) {
        if let Ok(mut guard) = self.etag_cache.lock() {
            guard.clear();
        }
    }

    pub fn invalidate_issue_cache(&self, issue_id: &IssueId) {
        if let Ok(mut guard) = self.etag_cache.lock() {
            let needle = format!("/issues/{}/", issue_id.as_str());
            guard.retain(|k, _| !k.as_str().contains(&needle) && !k.as_str().contains("/issues/?"));
        }
    }

    pub fn config(&self) -> &ClientConfig {
        &self.config
    }

    fn token(&self) -> Result<&str, ClientError> {
        if self.config.offline {
            return Err(ClientError::OfflineMode);
        }
        self.config
            .auth_token
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .ok_or(ClientError::Authentication)
    }

    fn base_url(&self) -> String {
        self.config.base_url.trim_end_matches('/').to_string()
    }

    fn attach_validators(
        &self,
        cache_key: &str,
        mut req: reqwest::RequestBuilder,
    ) -> reqwest::RequestBuilder {
        if let Ok(guard) = self.etag_cache.lock() {
            if let Some(v) = guard.get(cache_key) {
                if let Some(ref etag) = v.etag {
                    req = req.header(IF_NONE_MATCH, etag.as_str());
                }
                if let Some(ref lm) = v.last_modified {
                    req = req.header(IF_MODIFIED_SINCE, lm.as_str());
                }
            }
        }
        req
    }

    fn record_validators(&self, cache_key: &str, headers: &HeaderMap) {
        let etag = headers
            .get(ETAG)
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(CompactString::from);
        let last_modified = headers
            .get(LAST_MODIFIED)
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(CompactString::from);

        if etag.is_none() && last_modified.is_none() {
            return;
        }

        if let Ok(mut guard) = self.etag_cache.lock() {
            if guard.len() >= MAX_ETAG_CACHE_ENTRIES && !guard.contains_key(cache_key) {
                if let Some(evict_key) = guard.keys().next().cloned() {
                    guard.remove(&evict_key);
                }
            }
            guard.insert(
                CompactString::from(cache_key),
                ConditionalValidators {
                    etag,
                    last_modified,
                },
            );
        }
    }

    pub async fn fetch_projects(&self) -> Result<Vec<SentryProject>, ClientError> {
        Ok(self
            .fetch_projects_conditional()
            .await?
            .unwrap_or_default())
    }

    pub async fn fetch_projects_conditional(
        &self,
    ) -> Result<Option<Vec<SentryProject>>, ClientError> {
        let token = self.token()?;
        let url = format!(
            "{}/api/0/organizations/{}/projects/",
            self.base_url(),
            self.config.org
        );
        let req = self.attach_validators(&url, self.http.get(&url).bearer_auth(token));
        let resp = req
            .send()
            .await
            .map_err(|e| ClientError::Http(e.to_string().into()))?;

        let Some(val) = self.check_and_json_conditional(&url, resp).await? else {
            return Ok(None);
        };
        parse_projects_json(&val).map(Some)
    }

    pub async fn fetch_issues(
        &self,
        query: Option<&str>,
    ) -> Result<Vec<SentryIssue>, ClientError> {
        Ok(self
            .fetch_issues_conditional(query)
            .await?
            .unwrap_or_default())
    }

    pub async fn fetch_issues_conditional(
        &self,
        query: Option<&str>,
    ) -> Result<Option<Vec<SentryIssue>>, ClientError> {
        let token = self.token()?;
        let url = format!(
            "{}/api/0/organizations/{}/issues/",
            self.base_url(),
            self.config.org
        );
        let limit_str = self.config.max_issues_per_poll.min(100).to_string();
        let trimmed_q = query.map(str::trim).filter(|s| !s.is_empty());
        let cache_key = match trimmed_q {
            Some(q) => format!("{url}?limit={limit_str}&statsPeriod=24h&query={q}"),
            None => format!("{url}?limit={limit_str}&statsPeriod=24h"),
        };

        let mut req = self
            .http
            .get(&url)
            .bearer_auth(token)
            .query(&[("limit", limit_str.as_str()), ("statsPeriod", "24h")]);

        if let Some(q) = trimmed_q {
            req = req.query(&[("query", q)]);
        }
        req = self.attach_validators(&cache_key, req);

        let resp = req
            .send()
            .await
            .map_err(|e| ClientError::Http(e.to_string().into()))?;

        let Some(val) = self.check_and_json_conditional(&cache_key, resp).await? else {
            return Ok(None);
        };
        parse_issues_json(&val).map(Some)
    }

    pub async fn fetch_latest_event(
        &self,
        issue_id: &IssueId,
    ) -> Result<SentryEventDetail, ClientError> {
        let token = self.token()?;
        let url = format!(
            "{}/api/0/organizations/{}/issues/{}/events/latest/",
            self.base_url(),
            self.config.org,
            issue_id.as_str()
        );
        let resp = self
            .http
            .get(&url)
            .bearer_auth(token)
            .send()
            .await
            .map_err(|e| ClientError::Http(e.to_string().into()))?;

        let val = Self::check_and_json(resp).await?;
        parse_event_detail_json(issue_id, &val)
    }

    pub async fn fetch_latest_event_conditional(
        &self,
        issue_id: &IssueId,
    ) -> Result<Option<SentryEventDetail>, ClientError> {
        let token = self.token()?;
        let url = format!(
            "{}/api/0/organizations/{}/issues/{}/events/latest/",
            self.base_url(),
            self.config.org,
            issue_id.as_str()
        );
        let req = self.attach_validators(&url, self.http.get(&url).bearer_auth(token));
        let resp = req
            .send()
            .await
            .map_err(|e| ClientError::Http(e.to_string().into()))?;

        let Some(val) = self.check_and_json_conditional(&url, resp).await? else {
            return Ok(None);
        };
        parse_event_detail_json(issue_id, &val).map(Some)
    }

    pub async fn update_issue_status(
        &self,
        issue_id: &IssueId,
        status: IssueStatus,
    ) -> Result<IssueStatus, ClientError> {
        let token = self.token()?;
        let url = format!(
            "{}/api/0/organizations/{}/issues/{}/",
            self.base_url(),
            self.config.org,
            issue_id.as_str()
        );
        let body = json!({
            "status": status.as_api_str()
        });
        let resp = self
            .http
            .put(&url)
            .bearer_auth(token)
            .json(&body)
            .send()
            .await
            .map_err(|e| ClientError::Http(e.to_string().into()))?;

        let val = Self::check_and_json(resp).await?;
        self.invalidate_issue_cache(issue_id);
        let new_status = val
            .get("status")
            .and_then(|v| v.as_str())
            .map(IssueStatus::from_str_lossy)
            .unwrap_or(status);
        Ok(new_status)
    }

    async fn check_and_json_conditional(
        &self,
        cache_key: &str,
        resp: reqwest::Response,
    ) -> Result<Option<Value>, ClientError> {
        let status = resp.status();
        if status == StatusCode::NOT_MODIFIED {
            return Ok(None);
        }
        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            return Err(ClientError::Authentication);
        }
        if status == StatusCode::TOO_MANY_REQUESTS {
            return Err(ClientError::RateLimited);
        }
        let headers = resp.headers().clone();
        if !status.is_success() {
            let bytes = resp
                .bytes()
                .await
                .map_err(|e| ClientError::Http(e.to_string().into()))?;
            let decoded = decode_response_bytes(&headers, &bytes);
            let text = String::from_utf8_lossy(&decoded);
            return Err(ClientError::Api {
                status: status.as_u16(),
                message: text.chars().take(200).collect::<String>().into(),
            });
        }
        self.record_validators(cache_key, &headers);
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| ClientError::Http(e.to_string().into()))?;
        let decoded = decode_response_bytes(&headers, &bytes);
        serde_json::from_slice::<Value>(&decoded)
            .map(Some)
            .map_err(|e| ClientError::Parse(e.to_string().into()))
    }

    async fn check_and_json(resp: reqwest::Response) -> Result<Value, ClientError> {
        let status = resp.status();
        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            return Err(ClientError::Authentication);
        }
        if status == StatusCode::TOO_MANY_REQUESTS {
            return Err(ClientError::RateLimited);
        }
        let headers = resp.headers().clone();
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| ClientError::Http(e.to_string().into()))?;
        let decoded = decode_response_bytes(&headers, &bytes);
        if !status.is_success() {
            let text = String::from_utf8_lossy(&decoded);
            return Err(ClientError::Api {
                status: status.as_u16(),
                message: text.chars().take(200).collect::<String>().into(),
            });
        }
        serde_json::from_slice::<Value>(&decoded)
            .map_err(|e| ClientError::Parse(e.to_string().into()))
    }
}

fn decode_response_bytes(headers: &HeaderMap, raw: &[u8]) -> Vec<u8> {
    let encoding = headers
        .get(CONTENT_ENCODING)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_ascii_lowercase())
        .unwrap_or_default();

    match encoding.as_str() {
        "gzip" | "x-gzip" => {
            let mut decoder = GzDecoder::new(raw);
            let mut out = Vec::with_capacity(raw.len().saturating_mul(4));
            if decoder.read_to_end(&mut out).is_ok() {
                return out;
            }
            raw.to_vec()
        }
        "deflate" => {
            let mut zlib = ZlibDecoder::new(raw);
            let mut out = Vec::with_capacity(raw.len().saturating_mul(4));
            if zlib.read_to_end(&mut out).is_ok() {
                return out;
            }
            out.clear();
            let mut deflate = DeflateDecoder::new(raw);
            if deflate.read_to_end(&mut out).is_ok() {
                return out;
            }
            raw.to_vec()
        }
        _ => raw.to_vec(),
    }
}

pub fn parse_projects_json(val: &Value) -> Result<Vec<SentryProject>, ClientError> {
    let arr = val
        .as_array()
        .ok_or_else(|| ClientError::Parse("Expected array of projects".into()))?;
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        let id = item
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let slug = item
            .get("slug")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if id.is_empty() || slug.is_empty() {
            continue;
        }
        let name = item
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or(slug);
        let platform = item
            .get("platform")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(CompactString::from);
        let team_slug = item
            .get("team")
            .and_then(|t| t.get("slug"))
            .and_then(|v| v.as_str())
            .or_else(|| {
                item.get("teams")
                    .and_then(|t| t.as_array())
                    .and_then(|a| a.first())
                    .and_then(|t| t.get("slug"))
                    .and_then(|v| v.as_str())
            })
            .map(CompactString::from);

        out.push(SentryProject {
            id: ProjectId::from(id),
            slug: CompactString::from(slug),
            name: CompactString::from(name),
            platform,
            team_slug,
        });
    }
    Ok(out)
}

pub fn parse_issues_json(val: &Value) -> Result<Vec<SentryIssue>, ClientError> {
    let arr = val
        .as_array()
        .ok_or_else(|| ClientError::Parse("Expected array of issues".into()))?;
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        if let Some(issue) = parse_single_issue(item) {
            out.push(issue);
        }
    }
    Ok(out)
}

pub fn parse_single_issue(item: &Value) -> Option<SentryIssue> {
    let id = item.get("id")?.as_str()?;
    let short_id = item
        .get("shortId")
        .and_then(|v| v.as_str())
        .unwrap_or(id);
    let title = item
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("(Untitled Issue)");
    let culprit = item
        .get("culprit")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let permalink = item
        .get("permalink")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let level = item
        .get("level")
        .and_then(|v| v.as_str())
        .map(IssueLevel::from_str_lossy)
        .unwrap_or_default();
    let status = item
        .get("status")
        .and_then(|v| v.as_str())
        .map(IssueStatus::from_str_lossy)
        .unwrap_or_default();
    let substatus = item
        .get("substatus")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(CompactString::from);
    let priority = IssuePriority::from_str_opt(item.get("priority").and_then(|v| v.as_str()));
    let is_unhandled = item
        .get("isUnhandled")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let is_bookmarked = item
        .get("isBookmarked")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let has_seen = item
        .get("hasSeen")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let parse_u64_val = |v: &Value| {
        v.as_u64()
            .or_else(|| v.as_str().and_then(|s| s.parse::<u64>().ok()))
    };
    let count = item
        .get("lifetime")
        .and_then(|lt| lt.get("count"))
        .and_then(parse_u64_val)
        .or_else(|| item.get("count").and_then(parse_u64_val))
        .unwrap_or(0);
    let user_count = item
        .get("lifetime")
        .and_then(|lt| lt.get("userCount"))
        .and_then(parse_u64_val)
        .or_else(|| item.get("userCount").and_then(parse_u64_val))
        .unwrap_or(0);

    let first_seen = item
        .get("firstSeen")
        .and_then(|v| v.as_str())
        .and_then(parse_timestamp)
        .unwrap_or_else(Utc::now);
    let last_seen = item
        .get("lastSeen")
        .and_then(|v| v.as_str())
        .and_then(parse_timestamp)
        .unwrap_or(first_seen);

    let proj_obj = item.get("project");
    let project_id = proj_obj
        .and_then(|p| p.get("id"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let project_slug = proj_obj
        .and_then(|p| p.get("slug"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let project_name = proj_obj
        .and_then(|p| p.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or(project_slug);

    let platform = item
        .get("platform")
        .and_then(|v| v.as_str())
        .or_else(|| proj_obj.and_then(|p| p.get("platform")).and_then(|v| v.as_str()))
        .filter(|s| !s.is_empty())
        .map(CompactString::from);
    let logger = item
        .get("logger")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(CompactString::from);

    let meta = item.get("metadata");
    let error_type = meta
        .and_then(|m| m.get("type"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(CompactString::from);
    let error_value = meta
        .and_then(|m| m.get("value"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(CompactString::from);
    let sdk_name = meta
        .and_then(|m| m.get("sdk"))
        .and_then(|s| s.get("name"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(CompactString::from);

    let assigned_to = item
        .get("assignedTo")
        .and_then(|a| a.get("name").or_else(|| a.get("email")))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(CompactString::from);
    let num_comments = item
        .get("numComments")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;

    let mut stats_24h = Vec::new();
    if let Some(arr) = item
        .get("stats")
        .and_then(|s| s.get("24h"))
        .and_then(|v| v.as_array())
    {
        for pair in arr {
            if let Some(p) = pair.as_array() {
                if p.len() >= 2 {
                    let ts = p[0].as_i64().unwrap_or(0);
                    let cnt = p[1].as_u64().unwrap_or(0);
                    stats_24h.push((ts, cnt));
                }
            }
        }
    }

    let raw_json = serde_json::to_string_pretty(item).ok();

    Some(SentryIssue {
        id: IssueId::from(id),
        short_id: CompactString::from(short_id),
        title: CompactString::from(title),
        culprit: CompactString::from(culprit),
        permalink: CompactString::from(permalink),
        level,
        status,
        substatus,
        priority,
        is_unhandled,
        is_bookmarked,
        has_seen,
        count,
        user_count,
        first_seen,
        last_seen,
        project_id: ProjectId::from(project_id),
        project_slug: CompactString::from(project_slug),
        project_name: CompactString::from(project_name),
        platform,
        logger,
        error_type,
        error_value,
        sdk_name,
        assigned_to,
        num_comments,
        stats_24h,
        raw_json,
    })
}

pub fn parse_event_detail_json(
    issue_id: &IssueId,
    val: &Value,
) -> Result<SentryEventDetail, ClientError> {
    let event_id = val
        .get("eventID")
        .or_else(|| val.get("id"))
        .and_then(|v| v.as_str())
        .unwrap_or("latest");

    let date_created = val
        .get("dateCreated")
        .and_then(|v| v.as_str())
        .and_then(parse_timestamp)
        .unwrap_or_else(Utc::now);
    let date_received = val
        .get("dateReceived")
        .and_then(|v| v.as_str())
        .and_then(parse_timestamp);

    let title = val
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let message = val
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let culprit = val
        .get("culprit")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let platform = val
        .get("platform")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(CompactString::from);

    let release = val.get("release").and_then(|r| {
        r.as_str()
            .or_else(|| r.get("version").and_then(|v| v.as_str()))
            .filter(|s| !s.is_empty())
            .map(CompactString::from)
    });

    let size_bytes = val.get("size").and_then(|v| v.as_u64());

    let fingerprints = val
        .get("fingerprints")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(CompactString::from))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let mut tags = Vec::new();
    let mut environment = None;
    if let Some(tag_arr) = val.get("tags").and_then(|v| v.as_array()) {
        for t in tag_arr {
            let key = t
                .get("key")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            let value = t
                .get("value")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            if !key.is_empty() {
                if key == "environment" && !value.is_empty() {
                    environment = Some(CompactString::from(value));
                }
                tags.push(EventTag {
                    key: CompactString::from(key),
                    value: CompactString::from(value),
                });
            }
        }
    }

    let mut exceptions = Vec::new();
    let mut formatted_message = None;
    let mut breadcrumbs = Vec::new();
    let mut request = None;

    if let Some(entries) = val.get("entries").and_then(|v| v.as_array()) {
        for entry in entries {
            let entry_type = entry
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            let Some(data) = entry.get("data") else {
                continue;
            };
            match entry_type {
                "exception" => {
                    if let Some(values) = data.get("values").and_then(|v| v.as_array()) {
                        for exc in values {
                            exceptions.push(parse_exception_value(exc));
                        }
                    }
                }
                "message" => {
                    if let Some(fmt_msg) = data
                        .get("formatted")
                        .or_else(|| data.get("message"))
                        .and_then(|v| v.as_str())
                    {
                        if !fmt_msg.trim().is_empty() {
                            formatted_message = Some(fmt_msg.to_string());
                        }
                    }
                }
                "breadcrumbs" => {
                    if let Some(values) = data.get("values").and_then(|v| v.as_array()) {
                        for crumb in values {
                            breadcrumbs.push(parse_breadcrumb(crumb));
                        }
                    }
                }
                "request" => {
                    request = Some(parse_request_entry(data));
                }
                "threads" => {
                    // If there are no exception entries, extract frames from crashed/current threads
                    if exceptions.is_empty() {
                        if let Some(values) = data.get("values").and_then(|v| v.as_array()) {
                            for th in values {
                                let frames = th
                                    .get("stacktrace")
                                    .or_else(|| th.get("rawStacktrace"))
                                    .and_then(|s| s.get("frames"))
                                    .and_then(|f| f.as_array())
                                    .map(|arr| arr.iter().map(parse_stack_frame).collect::<Vec<_>>())
                                    .unwrap_or_default();
                                if !frames.is_empty() {
                                    let name = th
                                        .get("name")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("Thread");
                                    exceptions.push(ExceptionValue {
                                        exc_type: CompactString::from(format!("Thread ({name})")),
                                        value: String::new(),
                                        module: None,
                                        mechanism_type: None,
                                        handled: None,
                                        frames,
                                    });
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    let user = val.get("user").and_then(parse_user_context);

    let mut custom_context = Vec::new();
    if let Some(ctx_obj) = val.get("context").and_then(|v| v.as_object()) {
        for (k, v) in ctx_obj {
            custom_context.push((CompactString::from(k.as_str()), format_json_value(v)));
        }
    }

    let mut runtime_contexts = Vec::new();
    if let Some(ctxs_obj) = val.get("contexts").and_then(|v| v.as_object()) {
        for (ctx_name, ctx_val) in ctxs_obj {
            if let Some(inner) = ctx_val.as_object() {
                let mut fields = Vec::new();
                for (k, v) in inner {
                    if k == "type" || v.is_null() {
                        continue;
                    }
                    fields.push((CompactString::from(k.as_str()), format_json_value(v)));
                }
                if !fields.is_empty() {
                    runtime_contexts.push((CompactString::from(ctx_name.as_str()), fields));
                }
            }
        }
    }

    let sdk_info = val.get("sdk").and_then(|sdk| {
        let name = sdk.get("name").and_then(|v| v.as_str())?;
        let ver = sdk
            .get("version")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if ver.is_empty() {
            Some(CompactString::from(name))
        } else {
            Some(CompactString::from(format!("{name} v{ver}")))
        }
    });

    let mut packages = Vec::new();
    if let Some(pkg_obj) = val.get("packages").and_then(|v| v.as_object()) {
        for (k, v) in pkg_obj {
            if let Some(ver) = v.as_str() {
                packages.push((CompactString::from(k.as_str()), CompactString::from(ver)));
            }
        }
    }

    let raw_json = serde_json::to_string_pretty(val).unwrap_or_else(|_| val.to_string());

    Ok(SentryEventDetail {
        issue_id: issue_id.clone(),
        event_id: EventId::from(event_id),
        date_created,
        date_received,
        title: CompactString::from(title),
        message,
        culprit: CompactString::from(culprit),
        platform,
        release,
        environment,
        size_bytes,
        fingerprints,
        tags,
        exceptions,
        formatted_message,
        breadcrumbs,
        request,
        user,
        custom_context,
        runtime_contexts,
        sdk_info,
        packages,
        raw_json,
    })
}

fn parse_exception_value(exc: &Value) -> ExceptionValue {
    let exc_type = exc
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("Exception");
    let value = exc
        .get("value")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let module = exc
        .get("module")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(CompactString::from);

    let mech = exc.get("mechanism");
    let mechanism_type = mech
        .and_then(|m| m.get("type"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(CompactString::from);
    let handled = mech.and_then(|m| m.get("handled")).and_then(|v| v.as_bool());

    let frames = exc
        .get("stacktrace")
        .or_else(|| exc.get("rawStacktrace"))
        .and_then(|s| s.get("frames"))
        .and_then(|f| f.as_array())
        .map(|arr| arr.iter().map(parse_stack_frame).collect::<Vec<_>>())
        .unwrap_or_default();

    ExceptionValue {
        exc_type: CompactString::from(exc_type),
        value,
        module,
        mechanism_type,
        handled,
        frames,
    }
}

fn parse_stack_frame(f: &Value) -> StackFrame {
    let filename = f
        .get("filename")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(CompactString::from);
    let abs_path = f
        .get("absPath")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(CompactString::from);
    let module = f
        .get("module")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(CompactString::from);
    let function = f
        .get("function")
        .or_else(|| f.get("rawFunction"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(CompactString::from);
    let line_no = f.get("lineNo").and_then(|v| v.as_i64());
    let col_no = f.get("colNo").and_then(|v| v.as_i64());
    let in_app = f.get("inApp").and_then(|v| v.as_bool()).unwrap_or(false);
    let package = f
        .get("package")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(CompactString::from);

    let mut context_lines = Vec::new();
    if let Some(ctx_arr) = f.get("context").and_then(|v| v.as_array()) {
        for item in ctx_arr {
            if let Some(pair) = item.as_array() {
                if pair.len() >= 2 {
                    let ln = pair[0].as_i64().unwrap_or(0);
                    let code = pair[1].as_str().unwrap_or_default().to_string();
                    context_lines.push((ln, code));
                }
            }
        }
    }

    let mut vars = Vec::new();
    if let Some(vars_obj) = f.get("vars").and_then(|v| v.as_object()) {
        for (k, v) in vars_obj {
            vars.push((CompactString::from(k.as_str()), format_json_value(v)));
        }
    }

    StackFrame {
        filename,
        abs_path,
        module,
        function,
        line_no,
        col_no,
        in_app,
        package,
        context_lines,
        vars,
    }
}

fn parse_breadcrumb(c: &Value) -> BreadcrumbEntry {
    let timestamp = c
        .get("timestamp")
        .and_then(|v| v.as_str())
        .map(CompactString::from);
    let crumb_type = c
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("default")
        .into();
    let category = c
        .get("category")
        .and_then(|v| v.as_str())
        .unwrap_or("default")
        .into();
    let level = c
        .get("level")
        .and_then(|v| v.as_str())
        .unwrap_or("info")
        .into();
    let message = c
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();

    let mut data_fields = Vec::new();
    if let Some(obj) = c.get("data").and_then(|v| v.as_object()) {
        for (k, v) in obj {
            if !v.is_null() {
                data_fields.push((CompactString::from(k.as_str()), format_json_value(v)));
            }
        }
    }

    BreadcrumbEntry {
        timestamp,
        crumb_type,
        category,
        level,
        message,
        data_fields,
    }
}

fn parse_request_entry(data: &Value) -> RequestEntry {
    let method = data
        .get("method")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(CompactString::from);
    let url = data
        .get("url")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from);
    let inferred_content_type = data
        .get("inferredContentType")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(CompactString::from);
    let fragment = data
        .get("fragment")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from);

    let data_body = data.get("data").and_then(|v| {
        if v.is_null() {
            None
        } else if let Some(s) = v.as_str() {
            if s.is_empty() {
                None
            } else {
                Some(s.to_string())
            }
        } else {
            serde_json::to_string_pretty(v).ok()
        }
    });

    let query = parse_kv_pairs(data.get("query"));
    let headers = parse_kv_pairs(data.get("headers"));
    let cookies = parse_kv_pairs(data.get("cookies"));
    let env = parse_kv_pairs(data.get("env"));

    RequestEntry {
        method,
        url,
        inferred_content_type,
        fragment,
        data_body,
        query,
        headers,
        cookies,
        env,
    }
}

fn parse_kv_pairs(val: Option<&Value>) -> Vec<(CompactString, String)> {
    let Some(v) = val else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if let Some(arr) = v.as_array() {
        for item in arr {
            if let Some(pair) = item.as_array() {
                if pair.len() >= 2 {
                    let k = pair[0].as_str().unwrap_or_default();
                    let val_str = format_json_value(&pair[1]);
                    if !k.is_empty() {
                        out.push((CompactString::from(k), val_str));
                    }
                }
            }
        }
    } else if let Some(obj) = v.as_object() {
        for (k, val_item) in obj {
            if !val_item.is_null() {
                out.push((CompactString::from(k.as_str()), format_json_value(val_item)));
            }
        }
    }
    out
}

fn parse_user_context(u: &Value) -> Option<UserContext> {
    let obj = u.as_object()?;
    let id = obj
        .get("id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(CompactString::from);
    let email = obj
        .get("email")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(CompactString::from);
    let username = obj
        .get("username")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(CompactString::from);
    let ip_address = obj
        .get("ip_address")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(CompactString::from);
    let name = obj
        .get("name")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(CompactString::from);

    let geo = obj.get("geo").and_then(|g| {
        let city = g.get("city").and_then(|v| v.as_str()).unwrap_or_default();
        let region = g.get("region").and_then(|v| v.as_str()).unwrap_or_default();
        let cc = g
            .get("country_code")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let parts: Vec<&str> = [city, region, cc]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect();
        if parts.is_empty() {
            None
        } else {
            Some(CompactString::from(parts.join(", ")))
        }
    });

    if id.is_none()
        && email.is_none()
        && username.is_none()
        && ip_address.is_none()
        && name.is_none()
        && geo.is_none()
    {
        return None;
    }

    Some(UserContext {
        id,
        email,
        username,
        ip_address,
        name,
        geo,
    })
}

fn format_json_value(v: &Value) -> String {
    match v {
        Value::Null => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        Value::Array(_) | Value::Object(_) => {
            serde_json::to_string(v).unwrap_or_else(|_| v.to_string())
        }
    }
}

fn parse_timestamp(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_issue_and_event_json() {
        let issue_json = serde_json::json!([
            {
                "id": "7796091804",
                "shortId": "ACME-API-42",
                "title": "Oban.PerformError: App.ObanJobs.SyncWorker failed",
                "culprit": "lib/app/oban_jobs/cost_guard_job.ex",
                "permalink": "https://acme.sentry.io/issues/7796091804/",
                "level": "error",
                "status": "unresolved",
                "substatus": "ongoing",
                "priority": "high",
                "isUnhandled": true,
                "count": "5822",
                "userCount": 4,
                "firstSeen": "2026-10-16T13:00:05Z",
                "lastSeen": "2026-10-28T05:11:22Z",
                "project": {
                    "id": "4509427291193344",
                    "name": "elixir",
                    "slug": "acme-api",
                    "platform": "elixir"
                },
                "metadata": {
                    "type": "Oban.PerformError",
                    "value": "Upstream token refresh failed (400)",
                    "sdk": { "name": "sentry-elixir" }
                },
                "stats": {
                    "24h": [[1793080800, 28], [1793084400, 30]]
                }
            }
        ]);

        let issues = parse_issues_json(&issue_json).expect("valid issues");
        assert_eq!(issues.len(), 1);
        let iss = &issues[0];
        assert_eq!(iss.short_id.as_str(), "ACME-API-42");
        assert_eq!(iss.count, 5822);
        assert_eq!(iss.events_24h(), 58);
        assert!(iss.is_unhandled);
        assert_eq!(iss.error_type.as_deref(), Some("Oban.PerformError"));

        let event_json = serde_json::json!({
            "eventID": "d97a596029864efba244b0b69475a751",
            "dateCreated": "2026-10-28T05:11:22.778000Z",
            "title": "Oban.PerformError",
            "message": "",
            "culprit": "lib/app/oban_jobs/cost_guard_job.ex",
            "platform": "elixir",
            "tags": [
                { "key": "environment", "value": "production" },
                { "key": "oban_worker", "value": "App.ObanJobs.SyncWorker" }
            ],
            "context": {
                "attempt": 9,
                "oban_job_id": 376089
            },
            "contexts": {
                "os": { "name": "linux", "version": "6.8.0", "type": "os" }
            },
            "entries": [
                {
                    "type": "exception",
                    "data": {
                        "values": [
                            {
                                "type": "Oban.PerformError",
                                "value": "Upstream token refresh failed (400)",
                                "mechanism": { "type": "generic", "handled": true },
                                "stacktrace": {
                                    "frames": [
                                        {
                                            "filename": "lib/app/oban_jobs/cost_guard_job.ex",
                                            "function": "App.ObanJobs.CostGuardJob.perform/1",
                                            "lineNo": 111,
                                            "inApp": true,
                                            "context": [[110, "  def perform(job) do"], [111, "    raise \"boom\""]],
                                            "vars": { "job_id": 376089 }
                                        }
                                    ]
                                }
                            }
                        ]
                    }
                },
                {
                    "type": "request",
                    "data": {
                        "method": "POST",
                        "url": "http://127.0.0.1:4000/internal-api/v1/trajectory/ingest",
                        "headers": [["Content-Type", "application/json"]],
                        "data": "[REDACTED]"
                    }
                }
            ]
        });

        let ev = parse_event_detail_json(&iss.id, &event_json).expect("valid event detail");
        assert_eq!(ev.environment.as_deref(), Some("production"));
        assert_eq!(ev.total_frames(), 1);
        assert_eq!(ev.in_app_frames(), 1);
        assert_eq!(ev.custom_context.len(), 2);
        assert!(ev.request.is_some());
    }

    #[tokio::test]
    async fn test_etag_conditional_304_and_gzip_decompression() {
        use std::io::Write;
        use flate2::{Compression, write::GzEncoder};
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{header, method, path},
        };

        let server = MockServer::start().await;
        let payload = serde_json::json!([
            {
                "id": "7796091804",
                "shortId": "ACME-API-42",
                "title": "Oban.PerformError: App.ObanJobs.SyncWorker failed",
                "culprit": "lib/app/oban_jobs/cost_guard_job.ex",
                "permalink": "https://acme.sentry.io/issues/7796091804/",
                "level": "error",
                "status": "unresolved",
                "substatus": "ongoing",
                "priority": "high",
                "isUnhandled": true,
                "count": "5822",
                "userCount": 4,
                "firstSeen": "2026-10-16T13:00:05Z",
                "lastSeen": "2026-10-28T05:11:22Z",
                "project": {
                    "id": "4509427291193344",
                    "name": "elixir",
                    "slug": "acme-api",
                    "platform": "elixir"
                }
            }
        ])
        .to_string();

        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(payload.as_bytes()).unwrap();
        let compressed = encoder.finish().unwrap();

        Mock::given(method("GET"))
            .and(path("/api/0/organizations/nvestiv/issues/"))
            .and(header("if-none-match", "W/\"sentry-issues-etag-1\""))
            .respond_with(ResponseTemplate::new(304))
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/api/0/organizations/nvestiv/issues/"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "application/json")
                    .insert_header("content-encoding", "gzip")
                    .insert_header("etag", "W/\"sentry-issues-etag-1\"")
                    .set_body_raw(compressed, "application/json"),
            )
            .expect(1)
            .mount(&server)
            .await;

        let cfg = ClientConfig {
            base_url: server.uri().into(),
            org: "nvestiv".into(),
            auth_token: Some("test-token".into()),
            configured_projects: vec!["acme-api".into(), "iris-ex".into()],
            max_issues_per_poll: 100,
            offline: false,
        };
        let client = SentryApiClient::new(cfg).expect("client");

        // First call: receives gzipped 200 OK + ETag
        let first = client
            .fetch_issues_conditional(Some("project:[acme-api,iris-ex]"))
            .await
            .expect("first fetch")
            .expect("Some(issues)");
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].short_id.as_str(), "ACME-API-42");

        // Second call: sends If-None-Match and receives 304 Not Modified -> Ok(None)
        let second = client
            .fetch_issues_conditional(Some("project:[acme-api,iris-ex]"))
            .await
            .expect("second fetch");
        assert!(second.is_none());
    }
}
