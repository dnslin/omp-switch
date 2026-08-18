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

const SAFE_QUERY_NAMES: &[&str] = &[
    "alt",
    "api_version",
    "deployment",
    "format",
    "location",
    "project",
    "region",
    "version",
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum SecretKind {
    Authorization,
    Other,
}

pub(crate) fn redact_diagnostic(value: &str) -> String {
    if contains_structured_secret(value)
        || contains_encoded_secret(value)
        || contains_ambiguous_secret_assignment(value)
        || contains_unsafe_authorization(value)
        || contains_url_secret(value)
    {
        return "[诊断信息因可能包含凭据而已脱敏]".to_owned();
    }
    let tokens = diagnostic_tokens(value);
    let mut redacted = Vec::with_capacity(tokens.len().min(32));
    let mut index = 0;
    while index < tokens.len() && redacted.len() < 32 {
        let token = &tokens[index];
        let explicit_assignment = token.split_once(['=', ':']).is_some()
            || matches!(tokens.get(index + 1).map(String::as_str), Some("=" | ":"));
        if let Some((key, inline_value)) = token.split_once(['=', ':']) {
            if let Some(kind) = classify_secret_name(key) {
                redacted.push(format!("{key}=[已脱敏]"));
                index += assignment_width(kind, Some(inline_value), &tokens[index + 1..]);
                continue;
            }
        }
        let key = token.trim_end_matches(['=', ':']);
        if let Some(kind) = classify_secret_name(key) {
            let bearer_scheme = tokens
                .get(index + 1)
                .is_some_and(|value| value.eq_ignore_ascii_case("bearer"));
            if kind == SecretKind::Authorization && !explicit_assignment && !bearer_scheme {
                redacted.push(token.clone());
                index += 1;
                continue;
            }
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

fn contains_unsafe_authorization(value: &str) -> bool {
    let tokens = diagnostic_tokens(value);
    for (index, token) in tokens.iter().enumerate() {
        let inline_assignment = token.split_once(['=', ':']).is_some();
        let standalone_separator =
            matches!(tokens.get(index + 1).map(String::as_str), Some("=" | ":"));
        let explicit_assignment = inline_assignment || standalone_separator;
        if !token_contains_authorization_key(token) {
            continue;
        }
        if explicit_assignment || !authorization_tail_is_prose(&tokens[index + 1..]) {
            return true;
        }
    }
    false
}

fn authorization_tail_is_prose(tokens: &[String]) -> bool {
    !tokens.is_empty()
        && tokens.iter().all(|token| {
            let normalized = token.trim_matches(|character: char| character.is_ascii_punctuation());
            !normalized.is_empty()
                && !token.contains(['=', ':'])
                && !token.to_ascii_lowercase().starts_with("sk-")
                && !is_known_authorization_scheme(normalized)
                && classify_secret_name(normalized).is_none()
                && is_authorization_prose_word(normalized)
        })
}

fn is_known_authorization_scheme(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "basic" | "digest" | "negotiate" | "ntlm" | "aws4-hmac-sha256" | "hmac" | "signature"
    )
}

fn is_authorization_prose_word(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "authorization"
            | "check"
            | "checking"
            | "denied"
            | "error"
            | "failed"
            | "failure"
            | "for"
            | "from"
            | "header"
            | "headers"
            | "invalid"
            | "missing"
            | "not"
            | "permission"
            | "permissions"
            | "provider"
            | "request"
            | "requests"
            | "required"
            | "scheme"
            | "to"
            | "value"
            | "with"
            | "without"
    )
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
    if url.fragment().is_some() || path_contains_secret(url.path()) {
        return REDACTED_URL.to_owned();
    }

    let query_pairs = url
        .query_pairs()
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    if query_pairs
        .iter()
        .any(|(key, _)| !url_query_key_is_safe(key))
    {
        return REDACTED_URL.to_owned();
    }
    url.set_query(None);
    if !query_pairs.is_empty() {
        url.query_pairs_mut().extend_pairs(
            query_pairs
                .iter()
                .map(|(key, value)| (key.as_str(), value.as_str())),
        );
    }

    redact_diagnostic(url.as_str())
}

pub(crate) fn url_projection_is_lossless(value: &str) -> bool {
    let Ok(url) = Url::parse(value) else {
        return false;
    };
    if value.contains('@') && (!value.contains("://") || url.host_str().is_none()) {
        return false;
    }
    !url_candidate_contains_secret(value) && redact_diagnostic(value) == value
}

fn url_query_key_is_safe(key: &str) -> bool {
    let normalized = key.trim().to_ascii_lowercase().replace('-', "_");
    SAFE_QUERY_NAMES.contains(&normalized.as_str())
}

fn path_contains_secret(path: &str) -> bool {
    path.split('/').any(path_segment_contains_secret)
}

fn path_segment_contains_secret(segment: &str) -> bool {
    let mut decoded = segment.to_owned();
    for _ in 0..=2 {
        let normalized = decoded.to_ascii_lowercase();
        let canonical_name = normalized.replace('-', "_");
        let canonical_secret = canonical_path_contains_secret(&canonical_name);
        if normalized.starts_with("sk-")
            || canonical_secret
            || canonical_name.contains("api_key")
            || canonical_name.contains("access_token")
            || canonical_name.contains("refresh_token")
            || canonical_name.contains("client_secret")
            || canonical_name.contains("authorization")
            || canonical_name.contains("credential")
            || canonical_name.contains("password")
            || canonical_name.contains("signature")
        {
            return true;
        }
        if !decoded.contains('%') {
            return false;
        }
        decoded = match decode_percent_segment(&decoded) {
            Some(value) => value,
            None => return true,
        };
    }
    true
}

fn canonical_path_contains_secret(value: &str) -> bool {
    value
        .split(|character: char| {
            matches!(
                character,
                '=' | ':' | '/' | '?' | '&' | '#' | ';' | ',' | '.'
            )
        })
        .any(|part| {
            SECRET_NAMES
                .iter()
                .any(|name| has_path_marker_boundary(part, name))
                || SECRET_SUFFIXES.iter().any(|suffix| part.ends_with(suffix))
        })
}

fn has_path_marker_boundary(value: &str, marker: &str) -> bool {
    value == marker
        || value
            .strip_prefix(marker)
            .is_some_and(|rest| rest.starts_with('_'))
        || value
            .strip_suffix(marker)
            .is_some_and(|rest| rest.ends_with('_'))
}
fn decode_percent_segment(segment: &str) -> Option<String> {
    let bytes = segment.as_bytes();
    if !bytes.contains(&b'%') {
        return Some(segment.to_owned());
    }
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'%' {
            decoded.push(bytes[index]);
            index += 1;
            continue;
        }
        if index + 2 >= bytes.len() {
            return None;
        }
        let high = hex_digit(bytes[index + 1])?;
        let low = hex_digit(bytes[index + 2])?;
        decoded.push((high << 4) | low);
        index += 3;
    }
    String::from_utf8(decoded).ok()
}

fn hex_digit(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
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

fn contains_ambiguous_secret_assignment(value: &str) -> bool {
    contains_secret_assignment(value, false)
}

fn contains_secret_assignment(value: &str, include_simple_keys: bool) -> bool {
    let tokens = diagnostic_tokens(value);
    tokens.iter().enumerate().any(|(index, token)| {
        let standalone_separator =
            matches!(tokens.get(index + 1).map(String::as_str), Some("=" | ":"));
        (standalone_separator && is_secret_assignment_key(token, include_simple_keys))
            || token
                .match_indices(['=', ':'])
                .any(|(index, _)| is_secret_assignment_key(&token[..index], include_simple_keys))
    })
}

fn is_secret_assignment_key(key: &str, include_simple_keys: bool) -> bool {
    if include_simple_keys {
        key_component_secret_kind(key).is_some()
    } else {
        contains_ambiguous_secret_key(key)
    }
}

fn contains_encoded_secret(value: &str) -> bool {
    if !value.contains('%') {
        return false;
    }
    let mut decoded = value.to_owned();
    for _ in 0..=2 {
        decoded = match decode_percent_segment(&decoded) {
            Some(value) => value,
            None => return true,
        };
        if contains_secret_assignment(&decoded, true)
            || contains_unsafe_authorization(&decoded)
            || contains_url_secret(&decoded)
            || contains_structured_secret(&decoded)
        {
            return true;
        }
        if !decoded.contains('%') {
            break;
        }
    }
    false
}

fn contains_ambiguous_secret_key(key: &str) -> bool {
    classify_secret_name(key).is_none() && key_component_secret_kind(key).is_some()
}

fn key_component(value: &str) -> Option<&str> {
    value
        .rsplit(|character: char| {
            character.is_ascii_punctuation() && !matches!(character, '-' | '_' | '%')
        })
        .find(|component| !component.is_empty())
}

fn key_component_secret_kind(value: &str) -> Option<SecretKind> {
    let component = key_component(value)?;
    if let Some(kind) = classify_secret_name(component) {
        return Some(kind);
    }
    if !component.contains('%') {
        return None;
    }
    let mut decoded = component.to_owned();
    for _ in 0..=2 {
        decoded = decode_percent_segment(&decoded)?;
        let component = key_component(&decoded)?;
        if let Some(kind) = classify_secret_name(component) {
            return Some(kind);
        }
        if !component.contains('%') {
            return None;
        }
    }
    None
}

fn token_contains_authorization_key(token: &str) -> bool {
    token
        .match_indices(['=', ':'])
        .map(|(index, _)| &token[..index])
        .chain(std::iter::once(token))
        .any(|key| key_component_secret_kind(key) == Some(SecretKind::Authorization))
}

fn contains_structured_secret(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    let normalized = lower.replace('-', "_");
    let structured = value.contains('{')
        || value.contains('[')
        || lower.contains("http://")
        || lower.contains("https://")
        || lower.contains('?')
        || lower.contains('&');
    let named_secret = SECRET_NAMES
        .iter()
        .chain(SECRET_SUFFIXES)
        .any(|name| normalized.contains(name));
    structured && (named_secret || (lower.contains("sk-") && lower.contains("://")))
}

fn contains_url_secret(value: &str) -> bool {
    for token in diagnostic_tokens(value) {
        let candidate = trim_url_wrappers(&token);
        if url_candidate_contains_secret(candidate) {
            return true;
        }
        for (index, _) in candidate.match_indices(['=', ':']) {
            let value = trim_url_wrappers(&candidate[index + 1..]);
            if value != candidate && url_candidate_contains_secret(value) {
                return true;
            }
        }
    }
    false
}

fn trim_url_wrappers(value: &str) -> &str {
    value.trim_matches(|character: char| character.is_ascii_punctuation())
}

fn url_candidate_contains_secret(candidate: &str) -> bool {
    let Ok(url) = Url::parse(candidate) else {
        return candidate.contains("://");
    };
    !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || path_contains_secret(url.path())
        || url
            .query_pairs()
            .any(|(key, _)| !url_query_key_is_safe(key.as_ref()))
}
