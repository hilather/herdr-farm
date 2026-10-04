//! Version acceptance and provenance shared by native and OTLP adapters.
use std::cmp::Ordering;

#[derive(Eq, PartialEq, Ord, PartialOrd)]
enum Part { Numeric(u64), Text(String) }

fn parse(version: &str) -> Option<(Vec<u64>, Vec<Part>)> {
    let end = version.find(['-', '+']).unwrap_or(version.len());
    let numeric = version[..end].split('.').map(|p| {
        if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) { return None; }
        p.parse().ok()
    }).collect::<Option<Vec<u64>>>()?;
    if numeric.len() < 2 { return None; }
    let mut suffix = Vec::new();
    let rest = &version[end..];
    if !rest.is_empty() {
        let rest = &rest[1..];
        if rest.is_empty() || !rest.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'+')) { return None; }
        let groups: Vec<_> = rest.split('+').collect();
        if groups.len() > 2 || groups.iter().any(|g| g.split('.').any(|p|
            p.is_empty() || !p.bytes().any(|b| b.is_ascii_alphanumeric()))) { return None; }
        // Semver build metadata has no precedence; Muse's -R build does.
        if version.as_bytes()[end] == b'+' { return Some((numeric, suffix)); }
        let rest = rest.split('+').next()?;
        if rest.is_empty() { return None; }
        let mut start = 0;
        let bytes = rest.as_bytes();
        for i in 1..=bytes.len() {
            if i == bytes.len() || bytes[i].is_ascii_digit() != bytes[start].is_ascii_digit() {
                let part = &rest[start..i];
                suffix.push(if bytes[start].is_ascii_digit() { Part::Numeric(part.parse().ok()?) } else { Part::Text(part.into()) });
                start = i;
            }
        }
    }
    Some((numeric, suffix))
}

fn compare(a: &str, b: &str) -> Option<Ordering> {
    let (mut an, asuffix) = parse(a)?;
    let (mut bn, bsuffix) = parse(b)?;
    let len = an.len().max(bn.len());
    an.resize(len, 0); bn.resize(len, 0);
    Some(an.cmp(&bn).then_with(|| match (asuffix.is_empty(), bsuffix.is_empty()) {
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        _ => asuffix.cmp(&bsuffix),
    }))
}

pub(crate) fn accepted(version: &str, live: &[&str], fixture: &[&str]) -> bool {
    if live.is_empty() { return fixture.contains(&version); }
    live.iter().any(|v| compare(version, v).is_some_and(|c| c != Ordering::Less))
}

fn lists(version: &str) -> (&str, &'static [&'static str], &'static [&'static str]) {
    use super::codex::{self, claude, muse, opencode};
    match version.split_once('/') {
        Some(("claude-code", v)) => (v, claude::LIVE_VERSIONS, claude::FIXTURE_VERSIONS),
        Some(("muse", v)) => (v, muse::LIVE_VERSIONS, muse::FIXTURE_VERSIONS),
        Some(("grok", v)) => (v, &["1.0.46"], &["1.0.46"]),
        Some(("devin", v)) => (v, &["3000.11.3"], &["3000.11.3"]),
        Some(("opencode", v)) => (v, &[], opencode::FIXTURE_VERSIONS),
        Some((_, v)) => (v, &[], &[]),
        None => (version, codex::CERTIFIED, codex::CERTIFIED),
    }
}

pub(crate) fn nearest(version: &str) -> Option<&'static str> {
    let (v, live, _) = lists(version);
    if live.contains(&v) { return None; }
    live.iter().copied().filter(|c| compare(v, c).is_some_and(|o| o != Ordering::Less))
        .max_by(|a, b| compare(a, b).unwrap_or(Ordering::Equal))
}

pub(crate) fn provenance(version: &str) -> serde_json::Value {
    let (v, live, fixture) = lists(version);
    if let Some(nearest) = nearest(version) {
        serde_json::json!({"certification":"newer_than_certified", "nearest_certified_version":nearest})
    } else {
        serde_json::json!({"certification":if accepted(v, live, fixture) { if live.is_empty() { "fixture" } else { "live" } } else { "none" }})
    }
}

pub(crate) fn qualified_accepted(version: &str) -> bool {
    let (v, live, fixture) = lists(version);
    accepted(v, live, fixture)
}
