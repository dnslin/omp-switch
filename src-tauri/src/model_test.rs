use std::{
    error::Error,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use parking_lot::Mutex;
use reqwest::{
    Method,
    header::{AUTHORIZATION, HeaderMap, HeaderValue},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::{
    error::AppError,
    overview::{ModelTestConfiguration, OverviewAuthMode},
    provider_mutation::SupportedApi,
};

pub(crate) const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
pub(crate) const MAX_RESPONSE_BYTES: usize = 64 * 1024;

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ModelTestInput {
    pub(crate) provider_id: String,
    pub(crate) model_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModelTestResult {
    pub(crate) success: bool,
    pub(crate) provider_id: String,
    pub(crate) model_id: String,
    pub(crate) protocol: SupportedApi,
    pub(crate) latency_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) status: Option<u16>,
    pub(crate) message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) error_code: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModelTestTerminal {
    pub(crate) provider_id: String,
    pub(crate) model_id: String,
    pub(crate) message: String,
    pub(crate) error_code: String,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModelTestState {
    pub(crate) running: bool,
    pub(crate) provider_id: Option<String>,
    pub(crate) model_id: Option<String>,
    pub(crate) result: Option<ModelTestResult>,
    pub(crate) terminal: Option<ModelTestTerminal>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ModelTestBinding {
    pub(crate) target_path: String,
    pub(crate) models_hash: String,
}

#[derive(Clone)]
pub(crate) struct ModelTestCoordinator {
    state: Arc<Mutex<CoordinatorState>>,
    next_id: Arc<AtomicU64>,
}

#[derive(Default)]
struct CoordinatorState {
    active: Option<ActiveModelTest>,
    result: Option<ModelTestResult>,
    result_binding: Option<ModelTestBinding>,
    terminal: Option<ModelTestTerminal>,
}

struct ActiveModelTest {
    id: u64,
    provider_id: String,
    model_id: String,
    cancellation: CancellationToken,
    binding: Option<ModelTestBinding>,
    invalidated: bool,
    preparation_finished: bool,
    terminal_deferred: bool,
}

pub(crate) struct ModelTestGuard {
    coordinator: ModelTestCoordinator,
    id: u64,
    cancellation: CancellationToken,
    deferred_release: bool,
}

impl Default for ModelTestCoordinator {
    fn default() -> Self {
        Self {
            state: Arc::new(Mutex::new(CoordinatorState::default())),
            next_id: Arc::new(AtomicU64::new(0)),
        }
    }
}

impl ModelTestCoordinator {
    pub(crate) fn begin(
        &self,
        provider_id: &str,
        model_id: &str,
    ) -> Result<ModelTestGuard, AppError> {
        let mut state = self.state.lock();
        if state.active.is_some() {
            return Err(AppError::new(
                "model-test-busy",
                "已有模型测试正在进行。",
                "请等待当前测试完成或先取消当前测试。",
            ));
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let cancellation = CancellationToken::new();
        state.active = Some(ActiveModelTest {
            id,
            provider_id: provider_id.to_owned(),
            model_id: model_id.to_owned(),
            cancellation: cancellation.clone(),
            binding: None,
            invalidated: false,
            preparation_finished: false,
            terminal_deferred: false,
        });
        state.terminal = None;
        Ok(ModelTestGuard {
            coordinator: self.clone(),
            id,
            cancellation,
            deferred_release: false,
        })
    }

    pub(crate) fn cancel(&self) -> bool {
        let cancellation = self
            .state
            .lock()
            .active
            .as_ref()
            .map(|active| active.cancellation.clone());
        if let Some(cancellation) = cancellation {
            cancellation.cancel();
            true
        } else {
            false
        }
    }

    pub(crate) fn bind(&self, id: u64, binding: ModelTestBinding) {
        let mut state = self.state.lock();
        if let Some(active) = state.active.as_mut().filter(|active| active.id == id) {
            active.binding = Some(binding);
        }
    }
    pub(crate) fn defer_terminal(&self, id: u64, terminal: ModelTestTerminal) {
        let mut state = self.state.lock();
        let should_release =
            if let Some(active) = state.active.as_mut().filter(|active| active.id == id) {
                active.terminal_deferred = true;
                active.preparation_finished
            } else {
                false
            };
        state.result = None;
        state.result_binding = None;
        state.terminal = Some(terminal);
        if should_release {
            state.active = None;
        }
    }

    pub(crate) fn finish_preparation(&self, id: u64) {
        let mut state = self.state.lock();
        let should_release =
            if let Some(active) = state.active.as_mut().filter(|active| active.id == id) {
                active.preparation_finished = true;
                active.terminal_deferred
            } else {
                false
            };
        if should_release {
            state.active = None;
        }
    }

    pub(crate) fn invalidate(&self) {
        let mut state = self.state.lock();
        if let Some(active) = state.active.as_mut() {
            active.invalidated = true;
            active.cancellation.cancel();
        }
        state.result = None;
        state.result_binding = None;
        state.terminal = None;
    }

    pub(crate) fn invalidate_if_changed(&self, target_path: &str, models_hash: Option<&str>) {
        let mut state = self.state.lock();
        let active_changed = state
            .active
            .as_ref()
            .and_then(|active| active.binding.as_ref())
            .is_some_and(|binding| {
                binding.target_path != target_path
                    || Some(binding.models_hash.as_str()) != models_hash
            });
        let result_changed = state.result.is_some()
            && state.result_binding.as_ref().is_none_or(|binding| {
                binding.target_path != target_path
                    || Some(binding.models_hash.as_str()) != models_hash
            });
        if active_changed || result_changed {
            if let Some(active) = state.active.as_mut() {
                active.invalidated = true;
                active.cancellation.cancel();
            }
            state.result = None;
            state.result_binding = None;
            state.terminal = None;
        }
    }

    pub(crate) fn state(&self) -> ModelTestState {
        let state = self.state.lock();
        ModelTestState {
            running: state.active.is_some(),
            provider_id: state
                .active
                .as_ref()
                .map(|active| active.provider_id.clone()),
            model_id: state.active.as_ref().map(|active| active.model_id.clone()),
            result: state.result.clone(),
            terminal: state.terminal.clone(),
        }
    }

    fn complete(&self, id: u64, result: ModelTestResult, binding: Option<ModelTestBinding>) {
        let mut state = self.state.lock();
        if state.active.as_ref().is_some_and(|active| active.id == id) {
            let invalidated = state
                .active
                .as_ref()
                .is_some_and(|active| active.invalidated);
            state.active = None;
            if !invalidated {
                state.result = Some(result);
                state.result_binding = binding;
                state.terminal = None;
            }
        }
    }

    fn fail(&self, id: u64, terminal: ModelTestTerminal) {
        let mut state = self.state.lock();
        if state.active.as_ref().is_some_and(|active| active.id == id) {
            state.active = None;
            state.result = None;
            state.result_binding = None;
            state.terminal = Some(terminal);
        }
    }

    fn abandon(&self, id: u64) {
        let mut state = self.state.lock();
        if state.active.as_ref().is_some_and(|active| active.id == id) {
            state.active = None;
        }
    }
}

impl ModelTestGuard {
    pub(crate) fn cancellation(&self) -> CancellationToken {
        self.cancellation.clone()
    }
    pub(crate) fn id(&self) -> u64 {
        self.id
    }

    pub(crate) fn bind(&self, binding: ModelTestBinding) {
        self.coordinator.bind(self.id, binding);
    }

    pub(crate) fn complete(self, result: ModelTestResult, binding: Option<ModelTestBinding>) {
        self.coordinator.complete(self.id, result, binding);
    }
    pub(crate) fn fail(self, terminal: ModelTestTerminal) {
        self.coordinator.fail(self.id, terminal);
    }
    pub(crate) fn keep_lease(mut self) {
        self.deferred_release = true;
    }
}
impl Drop for ModelTestGuard {
    fn drop(&mut self) {
        if !self.deferred_release {
            self.coordinator.abandon(self.id);
        }
    }
}

struct PreparedRequest {
    method: Method,
    url: Url,
    headers: HeaderMap,
    body: Value,
    protocol: SupportedApi,
}

enum RequestFailure {
    Cancelled,
    Timeout,
    Dns,
    Tls,
    Connection,
    ResponseFormat,
}

#[cfg(test)]
pub(crate) async fn execute(
    configuration: ModelTestConfiguration,
    cancellation: CancellationToken,
    timeout: Duration,
) -> Result<ModelTestResult, AppError> {
    execute_until(configuration, cancellation, Instant::now() + timeout).await
}

pub(crate) async fn execute_until(
    configuration: ModelTestConfiguration,
    cancellation: CancellationToken,
    deadline: Instant,
) -> Result<ModelTestResult, AppError> {
    let request = build_request(&configuration)?;
    let started_at = Instant::now();
    if cancellation.is_cancelled() {
        return Ok(cancelled_result(&configuration, elapsed_ms(started_at)));
    }
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Ok(timeout_result(&configuration, elapsed_ms(started_at)));
    }
    let client = reqwest::Client::builder()
        .connect_timeout(remaining)
        .timeout(remaining)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| {
            AppError::new(
                "model-test-client",
                "无法初始化模型测试客户端。",
                "请重试；如果问题持续，请查看脱敏日志。",
            )
        })?;
    if Instant::now() >= deadline {
        return Ok(timeout_result(&configuration, elapsed_ms(started_at)));
    }
    let response_future = async move {
        let mut response = client
            .request(request.method, request.url)
            .headers(request.headers)
            .json(&request.body)
            .send()
            .await
            .map_err(|error| classify_request_error(&error))?;
        let status = response.status().as_u16();
        let body = if (200..300).contains(&status) {
            read_response_body(&mut response).await?
        } else {
            String::new()
        };
        Ok::<_, RequestFailure>((status, body))
    };
    let response = tokio::select! {
        _ = cancellation.cancelled() => Err(RequestFailure::Cancelled),
        _ = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)) => Err(RequestFailure::Timeout),
        response = response_future => response,
    };
    let latency_ms = elapsed_ms(started_at);
    match response {
        Ok((status, body)) => Ok(result_from_response(
            &configuration,
            request.protocol,
            latency_ms,
            status,
            &body,
        )),
        Err(RequestFailure::Cancelled) => Ok(cancelled_result(&configuration, latency_ms)),
        Err(failure) => Ok(network_failure_result(&configuration, latency_ms, failure)),
    }
}

async fn read_response_body(response: &mut reqwest::Response) -> Result<String, RequestFailure> {
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| classify_request_error(&error))?
    {
        if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(RequestFailure::ResponseFormat);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(String::from_utf8_lossy(&body).into_owned())
}

fn build_request(configuration: &ModelTestConfiguration) -> Result<PreparedRequest, AppError> {
    let protocol = configuration.protocol;
    let base_url = Url::parse(&configuration.base_url).map_err(|_| {
        AppError::new(
            "model-test-base-url",
            "Provider Base URL 无效。",
            "请在 Provider 详情修复 HTTP(S) Base URL 后重试。",
        )
    })?;
    if !matches!(base_url.scheme(), "http" | "https") || base_url.host_str().is_none() {
        return Err(AppError::new(
            "model-test-base-url",
            "Provider Base URL 无效。",
            "请在 Provider 详情修复 HTTP(S) Base URL 后重试。",
        ));
    }
    let (url, body) = match protocol {
        SupportedApi::OpenAiCompletions => (
            append_path(base_url, "chat/completions")?,
            json!({
                "model": configuration.model_id,
                "messages": [{"role": "user", "content": "OMP Switch model test"}],
                "max_tokens": 1,
            }),
        ),
        SupportedApi::OpenAiResponses => (
            append_path(base_url, "responses")?,
            json!({
                "model": configuration.model_id,
                "input": "OMP Switch model test",
                "max_output_tokens": 1,
            }),
        ),
        SupportedApi::AnthropicMessages => {
            let base_url = normalize_anthropic_base_url(base_url);
            (
                append_path(base_url, "v1/messages")?,
                json!({
                    "model": configuration.model_id,
                    "max_tokens": 1,
                    "messages": [{"role": "user", "content": "OMP Switch model test"}],
                }),
            )
        }
        SupportedApi::GoogleGenerativeAi => {
            let mut url = append_google_path(base_url, &configuration.model_id)?;
            let query = url
                .query_pairs()
                .filter(|(key, _)| key != "alt")
                .map(|(key, value)| (key.into_owned(), value.into_owned()))
                .collect::<Vec<_>>();
            {
                let mut query_pairs = url.query_pairs_mut();
                query_pairs.clear();
                for (key, value) in query {
                    query_pairs.append_pair(&key, &value);
                }
                query_pairs.append_pair("alt", "sse");
            }
            (
                url,
                json!({
                    "contents": [{"role": "user", "parts": [{"text": "OMP Switch model test"}]}],
                    "generationConfig": {"maxOutputTokens": 1},
                }),
            )
        }
    };
    let mut headers = HeaderMap::new();
    match configuration.auth_mode {
        OverviewAuthMode::ApiKey => {
            let api_key = configuration
                .api_key
                .as_deref()
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    AppError::new(
                        "model-test-authentication",
                        "Provider 没有可用的 Direct API Key。",
                        "请在 Provider 详情配置 API Key，或切换为无认证后重试。",
                    )
                })?;
            match protocol {
                SupportedApi::OpenAiCompletions | SupportedApi::OpenAiResponses => {
                    headers.insert(
                        AUTHORIZATION,
                        HeaderValue::from_str(&format!("Bearer {api_key}"))
                            .map_err(|_| invalid_authentication_error())?,
                    );
                }
                SupportedApi::AnthropicMessages => {
                    if is_official_anthropic_endpoint(&url) {
                        headers.insert(
                            reqwest::header::HeaderName::from_static("x-api-key"),
                            HeaderValue::from_str(api_key)
                                .map_err(|_| invalid_authentication_error())?,
                        );
                    } else {
                        headers.insert(
                            AUTHORIZATION,
                            HeaderValue::from_str(&format!("Bearer {api_key}"))
                                .map_err(|_| invalid_authentication_error())?,
                        );
                    }
                    headers.insert(
                        reqwest::header::HeaderName::from_static("anthropic-version"),
                        HeaderValue::from_static("2023-06-01"),
                    );
                }
                SupportedApi::GoogleGenerativeAi => {
                    headers.insert(
                        reqwest::header::HeaderName::from_static("x-goog-api-key"),
                        HeaderValue::from_str(api_key)
                            .map_err(|_| invalid_authentication_error())?,
                    );
                }
            }
        }
        OverviewAuthMode::None => {
            if protocol == SupportedApi::AnthropicMessages {
                headers.insert(
                    reqwest::header::HeaderName::from_static("anthropic-version"),
                    HeaderValue::from_static("2023-06-01"),
                );
            }
        }
        OverviewAuthMode::Unsupported => {
            return Err(AppError::new(
                "model-test-not-eligible",
                "当前 Provider 使用了不支持的认证配置。",
                "高级 Provider 不能发起模型测试。",
            ));
        }
    }
    if protocol == SupportedApi::GoogleGenerativeAi {
        headers.insert(
            reqwest::header::ACCEPT,
            HeaderValue::from_static("text/event-stream"),
        );
    }
    Ok(PreparedRequest {
        method: Method::POST,
        url,
        headers,
        body,
        protocol,
    })
}

fn normalize_anthropic_base_url(mut url: Url) -> Url {
    let path = url.path().trim_end_matches('/').to_owned();
    let normalized = path.strip_suffix("/v1").unwrap_or(&path);
    url.set_path(if normalized.is_empty() {
        "/"
    } else {
        normalized
    });
    url
}

fn is_official_anthropic_endpoint(url: &Url) -> bool {
    url.scheme() == "https"
        && url.port().is_none()
        && url
            .host_str()
            .is_some_and(|host| host.eq_ignore_ascii_case("api.anthropic.com"))
}

fn append_path(mut url: Url, suffix: &str) -> Result<Url, AppError> {
    let path = url.path().trim_end_matches('/');
    url.set_path(&format!("{path}/{suffix}"));
    Ok(url)
}

fn append_google_path(mut url: Url, model_id: &str) -> Result<Url, AppError> {
    {
        let mut segments = url.path_segments_mut().map_err(|_| {
            AppError::new(
                "model-test-base-url",
                "Provider Base URL 无效。",
                "请在 Provider 详情修复 HTTP(S) Base URL 后重试。",
            )
        })?;
        segments.pop_if_empty().push("models").push(model_id);
    }
    let path = url.path().to_owned();
    url.set_path(&format!("{path}:streamGenerateContent"));
    Ok(url)
}

fn result_from_response(
    configuration: &ModelTestConfiguration,
    protocol: SupportedApi,
    latency_ms: u64,
    status: u16,
    body: &str,
) -> ModelTestResult {
    if !(200..300).contains(&status) {
        let (error_code, message) = http_failure(status);
        return failure_result(
            configuration,
            protocol,
            latency_ms,
            Some(status),
            error_code,
            message,
        );
    }
    if !valid_response(protocol, body) {
        return failure_result(
            configuration,
            protocol,
            latency_ms,
            Some(status),
            "response-format",
            "Provider 返回了不符合协议的响应。",
        );
    }
    ModelTestResult {
        success: true,
        provider_id: configuration.provider_id.clone(),
        model_id: configuration.model_id.clone(),
        protocol,
        latency_ms,
        status: Some(status),
        message: "模型连接成功".to_owned(),
        error_code: None,
    }
}

fn valid_response(protocol: SupportedApi, body: &str) -> bool {
    let value = match protocol {
        SupportedApi::GoogleGenerativeAi => google_response_value(body),
        _ => serde_json::from_str::<Value>(body).ok(),
    };
    let Some(value) = value else {
        return false;
    };
    match protocol {
        SupportedApi::OpenAiCompletions => value.get("choices").is_some_and(Value::is_array),
        SupportedApi::OpenAiResponses => value.get("output").is_some_and(Value::is_array),
        SupportedApi::AnthropicMessages => value.get("content").is_some_and(Value::is_array),
        SupportedApi::GoogleGenerativeAi => value.get("candidates").is_some_and(Value::is_array),
    }
}

fn google_response_value(body: &str) -> Option<Value> {
    if let Ok(value) = serde_json::from_str::<Value>(body) {
        return Some(value);
    }
    body.lines().find_map(|line| {
        let data = line.strip_prefix("data:")?.trim();
        if data.is_empty() || data == "[DONE]" {
            return None;
        }
        serde_json::from_str::<Value>(data).ok()
    })
}

fn http_failure(status: u16) -> (&'static str, &'static str) {
    match status {
        401 => ("http-401", "Provider 拒绝了认证，请检查 API Key。"),
        403 => ("http-403", "Provider 拒绝了当前权限。"),
        404 => ("http-404", "Provider 没有找到请求的模型地址。"),
        429 => ("http-429", "Provider 请求频率或额度受限。"),
        500..=599 => ("http-5xx", "Provider 服务暂时异常。"),
        _ => ("http-status", "Provider 返回了未成功的 HTTP 状态。"),
    }
}

fn classify_request_error(error: &reqwest::Error) -> RequestFailure {
    if error.is_timeout() {
        return RequestFailure::Timeout;
    }
    let mut diagnostic = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        diagnostic.push(' ');
        diagnostic.push_str(&cause.to_string());
        source = cause.source();
    }
    let diagnostic = diagnostic.to_ascii_lowercase();
    if diagnostic.contains("dns")
        || diagnostic.contains("failed to lookup")
        || diagnostic.contains("name or service not known")
        || diagnostic.contains("nodename nor servname")
    {
        return RequestFailure::Dns;
    }
    if diagnostic.contains("tls")
        || diagnostic.contains("ssl")
        || diagnostic.contains("certificate")
    {
        return RequestFailure::Tls;
    }
    RequestFailure::Connection
}

fn network_failure_result(
    configuration: &ModelTestConfiguration,
    latency_ms: u64,
    failure: RequestFailure,
) -> ModelTestResult {
    let (error_code, message) = match failure {
        RequestFailure::Cancelled => ("cancelled", "测试已取消"),
        RequestFailure::Timeout => ("timeout", "请求超时，请检查网络或服务响应。"),
        RequestFailure::Dns => ("dns", "无法解析 Provider 地址，请检查域名或网络。"),
        RequestFailure::Tls => ("tls", "TLS 连接失败，请检查证书和 HTTPS 配置。"),
        RequestFailure::Connection => ("connection", "无法连接到 Provider，请检查网络或服务状态。"),
        RequestFailure::ResponseFormat => (
            "response-format",
            "Provider 返回了不符合协议的响应，请检查协议和响应格式。",
        ),
    };
    failure_result(
        configuration,
        configuration.protocol,
        latency_ms,
        None,
        error_code,
        message,
    )
}

fn cancelled_result(configuration: &ModelTestConfiguration, latency_ms: u64) -> ModelTestResult {
    failure_result(
        configuration,
        configuration.protocol,
        latency_ms,
        None,
        "cancelled",
        "测试已取消",
    )
}

fn timeout_result(configuration: &ModelTestConfiguration, latency_ms: u64) -> ModelTestResult {
    failure_result(
        configuration,
        configuration.protocol,
        latency_ms,
        None,
        "timeout",
        "请求超时，请检查网络或服务响应。",
    )
}

fn failure_result(
    configuration: &ModelTestConfiguration,
    protocol: SupportedApi,
    latency_ms: u64,
    status: Option<u16>,
    error_code: &str,
    message: &str,
) -> ModelTestResult {
    ModelTestResult {
        success: false,
        provider_id: configuration.provider_id.clone(),
        model_id: configuration.model_id.clone(),
        protocol,
        latency_ms,
        status,
        message: message.to_owned(),
        error_code: Some(error_code.to_owned()),
    }
}

fn invalid_authentication_error() -> AppError {
    AppError::new(
        "model-test-authentication",
        "Provider 的认证值无法用于 HTTP 请求。",
        "请在 Provider 详情重新配置 API Key。",
    )
}

fn elapsed_ms(started_at: Instant) -> u64 {
    started_at.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anthropic_configuration(base_url: &str) -> ModelTestConfiguration {
        ModelTestConfiguration {
            provider_id: "provider".to_owned(),
            model_id: "model".to_owned(),
            base_url: base_url.to_owned(),
            protocol: SupportedApi::AnthropicMessages,
            auth_mode: OverviewAuthMode::ApiKey,
            api_key: Some("saved-key".to_owned()),
            target_path: "/tmp/test-target".to_owned(),
            models_hash: "test-models-hash".to_owned(),
        }
    }

    #[test]
    fn chooses_anthropic_auth_by_endpoint_and_normalizes_version_path() {
        let official =
            build_request(&anthropic_configuration("https://api.anthropic.com/v1/")).unwrap();
        assert_eq!(official.url.path(), "/v1/messages");
        assert_eq!(official.headers["x-api-key"], "saved-key");
        assert!(!official.headers.contains_key(AUTHORIZATION));

        let custom = build_request(&anthropic_configuration("https://example.com/v1")).unwrap();
        assert_eq!(custom.url.path(), "/v1/messages");
        assert_eq!(custom.headers[AUTHORIZATION], "Bearer saved-key");
        assert!(!custom.headers.contains_key("x-api-key"));
    }

    #[test]
    fn keeps_results_when_bound_target_and_models_hash_stay_same() {
        let coordinator = ModelTestCoordinator::default();
        let guard = coordinator.begin("provider", "model").unwrap();
        let binding = ModelTestBinding {
            target_path: "/tmp/target".to_owned(),
            models_hash: "models-v1".to_owned(),
        };
        guard.bind(binding.clone());
        guard.complete(
            ModelTestResult {
                success: true,
                provider_id: "provider".to_owned(),
                model_id: "model".to_owned(),
                protocol: SupportedApi::OpenAiResponses,
                latency_ms: 12,
                status: Some(200),
                message: "模型连接成功".to_owned(),
                error_code: None,
            },
            Some(binding),
        );

        coordinator.invalidate_if_changed("/tmp/target", Some("models-v1"));

        assert!(coordinator.state().result.is_some());
        assert!(!coordinator.state().running);
    }

    #[test]
    fn invalidates_results_when_bound_target_or_models_hash_changes() {
        let coordinator = ModelTestCoordinator::default();
        let guard = coordinator.begin("provider", "model").unwrap();
        let binding = ModelTestBinding {
            target_path: "/tmp/target".to_owned(),
            models_hash: "models-v1".to_owned(),
        };
        guard.bind(binding.clone());
        guard.complete(
            ModelTestResult {
                success: true,
                provider_id: "provider".to_owned(),
                model_id: "model".to_owned(),
                protocol: SupportedApi::OpenAiResponses,
                latency_ms: 12,
                status: Some(200),
                message: "模型连接成功".to_owned(),
                error_code: None,
            },
            Some(binding),
        );
        assert!(coordinator.state().result.is_some());

        coordinator.invalidate_if_changed("/tmp/target", Some("models-v2"));

        assert!(coordinator.state().result.is_none());
        assert!(!coordinator.state().running);
    }
    #[test]
    fn invalidates_unbound_results_when_overview_refreshes() {
        let coordinator = ModelTestCoordinator::default();
        let guard = coordinator.begin("provider", "model").unwrap();
        guard.complete(
            ModelTestResult {
                success: false,
                provider_id: "provider".to_owned(),
                model_id: "model".to_owned(),
                protocol: SupportedApi::OpenAiResponses,
                latency_ms: 12,
                status: None,
                message: "测试已取消".to_owned(),
                error_code: Some("cancelled".to_owned()),
            },
            None,
        );
        assert!(coordinator.state().result.is_some());

        coordinator.invalidate_if_changed("/tmp/target", Some("models-v1"));

        assert!(coordinator.state().result.is_none());
    }

    #[test]
    fn invalidation_keeps_active_test_busy_until_guard_finishes() {
        let coordinator = ModelTestCoordinator::default();
        let guard = coordinator.begin("provider", "model").unwrap();
        coordinator.invalidate();

        assert_eq!(
            coordinator
                .begin("other-provider", "other-model")
                .err()
                .unwrap()
                .code,
            "model-test-busy"
        );
        drop(guard);
        assert!(!coordinator.state().running);
    }
}
