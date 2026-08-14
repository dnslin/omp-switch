use url::Url;
const REDACTED_URL: &str = "[配置地址因无法解析而已脱敏]";

const SECRET_NAMES: &[&str] = &[
    "api_key",
    "apikey",
    "token",
    "access_token",
    "refresh_token",
    "password",
    "passwd",
    "secret",
    "client_secret",
    "authorization",
    "proxy_authorization",
    "x_api_key",
];

const SECRET_SUFFIXES: &[&str] = &[
    "_api_key",
    "_apikey",
    "_access_token",
    "_refresh_token",
    "_password",
    "_passwd",
    "_client_secret",
    "_authorization",
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum SecretKind {
    Authorization,
    Other,
}

pub(crate) fn redact_diagnostic(value: &str) -> String {
    if contains_structured_secret(value) {
        return "[诊断信息因可能包含凭据而已脱敏]".to_owned();
    }
    let tokens = diagnostic_tokens(value);
    let mut redacted = Vec::with_capacity(tokens.len().min(32));
    let mut index = 0;
    while index < tokens.len() && redacted.len() < 32 {
        let token = &tokens[index];
        if let Some((key, inline_value)) = token.split_once(['=', ':']) {
            if let Some(kind) = classify_secret_name(key) {
                redacted.push(format!("{key}=[已脱敏]"));
                index += assignment_width(kind, Some(inline_value), &tokens[index + 1..]);
                continue;
            }
        }
        let key = token.trim_end_matches(['=', ':']);
        if let Some(kind) = classify_secret_name(key) {
            redacted.push(token.clone());
            redacted.push("[已脱敏]".to_owned());
            index += assignment_width(kind, None, &tokens[index + 1..]);
            continue;
        }
        if token.to_ascii_lowercase().starts_with("sk-") {
            redacted.push("[已脱敏]".to_owned());
        } else {
            redacted.push(token.clone());
        }
        index += 1;
    }
    redacted.join(" ")
}
pub(crate) fn redact_projection(value: &str) -> String {
    let Ok(mut url) = Url::parse(value) else {
        return REDACTED_URL.to_owned();
    };
    if value.contains('@') && (!value.contains("://") || url.host_str().is_none()) {
        return REDACTED_URL.to_owned();
    }
    let _ = url.set_username("");
    let _ = url.set_password(None);

    let safe_query_pairs = url
        .query_pairs()
        .filter(|(key, _)| !url_query_key_is_secret(key))
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    url.set_query(None);
    if !safe_query_pairs.is_empty() {
        url.query_pairs_mut().extend_pairs(
            safe_query_pairs
                .iter()
                .map(|(key, value)| (key.as_str(), value.as_str())),
        );
    }

    redact_diagnostic(url.as_str())
}

fn url_query_key_is_secret(key: &str) -> bool {
    const SECRET_QUERY_NAMES: &[&str] = &[
        "key",
        "api_key",
        "apikey",
        "token",
        "access_token",
        "refresh_token",
        "password",
        "passwd",
        "secret",
        "client_secret",
        "authorization",
        "proxy_authorization",
        "x_api_key",
    ];
    const SECRET_QUERY_SUFFIXES: &[&str] = &[
        "_api_key",
        "_apikey",
        "_access_token",
        "_refresh_token",
        "_password",
        "_passwd",
        "_client_secret",
        "_authorization",
    ];

    let normalized = key.trim().to_ascii_lowercase().replace('-', "_");
    SECRET_QUERY_NAMES.contains(&normalized.as_str())
        || SECRET_QUERY_SUFFIXES
            .iter()
            .any(|suffix| normalized.ends_with(suffix))
}

fn diagnostic_tokens(value: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    for character in value.chars() {
        match (quote, character) {
            (Some(active), value) if value == active => quote = None,
            (Some(_), value) => current.push(value),
            (None, '\'' | '"') => quote = Some(character),
            (None, value) if value.is_whitespace() => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            (None, value) => current.push(value),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn assignment_width(kind: SecretKind, inline_value: Option<&str>, following: &[String]) -> usize {
    match inline_value {
        Some(value) if !value.is_empty() => {
            1 + usize::from(
                kind == SecretKind::Authorization
                    && value.eq_ignore_ascii_case("bearer")
                    && !following.is_empty(),
            )
        }
        _ => 1 + separated_value_width(kind, following),
    }
}

fn separated_value_width(kind: SecretKind, following: &[String]) -> usize {
    let separator = usize::from(matches!(
        following.first().map(String::as_str),
        Some("=" | ":")
    ));
    let values = &following[separator..];
    if kind == SecretKind::Authorization
        && values
            .first()
            .is_some_and(|value| value.eq_ignore_ascii_case("bearer"))
    {
        return separator + values.len().min(2);
    }
    separator + usize::from(!values.is_empty())
}

fn classify_secret_name(value: &str) -> Option<SecretKind> {
    let normalized = value.trim().to_ascii_lowercase().replace('-', "_");
    let is_secret = SECRET_NAMES.contains(&normalized.as_str())
        || SECRET_SUFFIXES
            .iter()
            .any(|suffix| normalized.ends_with(suffix));
    if !is_secret {
        return None;
    }
    if normalized == "authorization"
        || normalized == "proxy_authorization"
        || normalized.ends_with("_authorization")
    {
        Some(SecretKind::Authorization)
    } else {
        Some(SecretKind::Other)
    }
}

fn contains_structured_secret(value: &str) -> bool {
    let lower = value.to_ascii_lowercase().replace('-', "_");
    let structured = value.contains('{')
        || value.contains('[')
        || lower.contains("http://")
        || lower.contains("https://")
        || lower.contains('?')
        || lower.contains('&');
    structured
        && SECRET_NAMES
            .iter()
            .chain(SECRET_SUFFIXES)
            .any(|name| lower.contains(name))
}
