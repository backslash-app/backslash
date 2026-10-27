//! Fuzzy ranking shared by every source (apps, commands, host-filtered lists),
//! boosted by what the user actually picks.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use serde::{Deserialize, Serialize};

use crate::store;

/// Something that can be searched.
pub trait Searchable {
    fn title(&self) -> &str;
    fn keywords(&self) -> &[String] {
        &[]
    }
    fn subtitle(&self) -> Option<&str> {
        None
    }
}

/// Usage history and how to compute a candidate's usage id.
pub type UsageKey<'a, T> = (&'a Usage, &'a dyn Fn(&T) -> String);

pub struct Ranker {
    matcher: Matcher,
    buf: Vec<char>,
}

impl Default for Ranker {
    fn default() -> Self {
        let mut config = Config::DEFAULT;
        config.prefer_prefix = true;
        Self { matcher: Matcher::new(config), buf: Vec::new() }
    }
}

impl Ranker {
    /// Returns `(index, score)` of matching candidates, best first. With an
    /// empty query everything matches and only usage decides the order.
    pub fn rank<T: Searchable>(
        &mut self,
        query: &str,
        candidates: &[T],
        usage: Option<UsageKey<'_, T>>,
    ) -> Vec<(usize, u32)> {
        let query = query.trim();
        let pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);
        let substring = Pattern::new(query, CaseMatching::Smart, Normalization::Smart, AtomKind::Substring);
        let matches: Vec<(usize, u32, bool)> = candidates
            .iter()
            .enumerate()
            .filter_map(|(i, c)| {
                let (base, exact) = if query.is_empty() { (0, true) } else { self.score(&pattern, &substring, c)? };
                Some((i, base, exact))
            })
            .collect();
        // Fuzzy matching finds letters scattered anywhere ("deno" in
        // "xDg-documENt-pOrtal"). Keep exact substrings, and fuzzy matches
        // that are close to the best one.
        let best = matches.iter().map(|m| m.1).max().unwrap_or(0);
        let mut scored: Vec<(usize, u32)> = matches
            .into_iter()
            .filter(|&(_, base, exact)| exact || base * 3 >= best * 2)
            .map(|(i, base, _)| {
                let boost = usage.map_or(0, |(usage, id)| usage.boost(&id(&candidates[i]), query));
                (i, base + boost)
            })
            .collect();
        // Stable sort keeps the source order (e.g. alphabetical) for ties.
        scored.sort_by_key(|&(_, score)| std::cmp::Reverse(score));
        scored
    }

    /// Best weighted score over title, keywords and subtitle, and whether
    /// the query appears verbatim (as a substring) in any of them.
    fn score<T: Searchable>(&mut self, fuzzy: &Pattern, substring: &Pattern, c: &T) -> Option<(u32, bool)> {
        let mut best = None::<u32>;
        let mut exact = false;
        let mut consider = |text: &str, weight_pct: u32, fuzzy_allowed: bool, this: &mut Self| {
            let haystack = Utf32Str::new(text, &mut this.buf);
            let is_exact = substring.score(haystack, &mut this.matcher).is_some();
            let haystack = Utf32Str::new(text, &mut this.buf);
            let score = if fuzzy_allowed {
                fuzzy.score(haystack, &mut this.matcher)
            } else {
                substring.score(haystack, &mut this.matcher)
            };
            if let Some(s) = score {
                exact |= is_exact;
                let s = s * weight_pct / 100;
                best = Some(best.map_or(s, |b| b.max(s)));
            }
        };
        consider(c.title(), 100, true, self);
        for keyword in c.keywords() {
            consider(keyword, 80, true, self);
        }
        // Subtitles are long (paths, descriptions): only exact substrings count.
        if let Some(subtitle) = c.subtitle() {
            consider(subtitle, 50, false, self);
        }
        best.map(|b| (b, exact))
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Entry {
    count: u32,
    last: u64,
}

/// Frecency (frequency × recency) plus "you typed `fi` and picked Firefox last time".
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Usage {
    #[serde(default)]
    entries: HashMap<String, Entry>,
    #[serde(default)]
    queries: HashMap<String, String>,
    #[serde(skip)]
    path: Option<PathBuf>,
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

impl Usage {
    pub fn load(path: PathBuf) -> Self {
        let mut usage: Usage = store::read_json(&path);
        usage.path = Some(path);
        usage
    }

    pub fn record(&mut self, id: &str, query: &str) {
        let entry = self.entries.entry(id.to_string()).or_default();
        entry.count = entry.count.saturating_add(1);
        entry.last = now();
        let query = query.trim().to_lowercase();
        if !query.is_empty() {
            self.queries.insert(query, id.to_string());
        }
        if self.queries.len() > 2000 {
            self.queries.clear();
        }
        if let Some(path) = &self.path {
            let _ = store::write_json(path, self);
        }
    }

    /// `(times used, last use as Unix time)`.
    pub fn stats(&self, id: &str) -> Option<(u32, u64)> {
        self.entries.get(id).map(|e| (e.count, e.last))
    }

    fn frecency(&self, id: &str) -> f64 {
        let Some(e) = self.entries.get(id) else { return 0.0 };
        let age = now().saturating_sub(e.last);
        let recency = match age {
            a if a < 3600 => 4.0,
            a if a < 86_400 => 2.0,
            a if a < 7 * 86_400 => 1.0,
            a if a < 30 * 86_400 => 0.5,
            _ => 0.25,
        };
        f64::from(e.count.min(50)) * recency
    }

    /// Score bonus, on the same scale as fuzzy scores (a good match is ~100–300).
    pub fn boost(&self, id: &str, query: &str) -> u32 {
        let mut bonus = (self.frecency(id) * 4.0).min(120.0) as u32;
        let query = query.trim().to_lowercase();
        if !query.is_empty() && self.queries.get(&query).is_some_and(|chosen| chosen == id) {
            bonus += 400;
        }
        bonus
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct C(&'static str, Vec<String>);
    impl Searchable for C {
        fn title(&self) -> &str {
            self.0
        }
        fn keywords(&self) -> &[String] {
            &self.1
        }
    }

    fn titles(ranked: &[(usize, u32)], c: &[C]) -> Vec<&'static str> {
        ranked.iter().map(|(i, _)| c[*i].0).collect()
    }

    #[test]
    fn fuzzy_prefers_prefix_and_uses_keywords() {
        let c = vec![
            C("Files", vec![]),
            C("Firefox", vec!["browser".into()]),
            C("System Monitor", vec!["task manager".into()]),
        ];
        let mut r = Ranker::default();
        assert_eq!(titles(&r.rank("fire", &c, None), &c), vec!["Firefox"]);
        assert_eq!(titles(&r.rank("brow", &c, None), &c), vec!["Firefox"]);
        assert_eq!(titles(&r.rank("task", &c, None), &c), vec!["System Monitor"]);
        assert_eq!(r.rank("", &c, None).len(), 3);
    }

    #[test]
    fn subtitles_need_a_substring() {
        struct S(&'static str, &'static str);
        impl Searchable for S {
            fn title(&self) -> &str {
                self.0
            }
            fn subtitle(&self) -> Option<&str> {
                Some(self.1)
            }
        }
        let c = [
            S("xdg-document-portal", "/usr/libexec/xdg-document-portal"),
            S("node", "deno run main.ts"),
            S("deno", ""),
        ];
        let ranked = Ranker::default().rank("deno", &c, None);
        let titles: Vec<_> = ranked.iter().map(|(i, _)| c[*i].0).collect();
        assert_eq!(titles, vec!["deno", "node"]);
    }

    #[test]
    fn usage_learns_query_choice() {
        let c = vec![C("Files", vec![]), C("Firefox", vec![])];
        let mut usage = Usage::default();
        let id = |c: &C| c.0.to_string();
        let mut r = Ranker::default();
        let first = titles(&r.rank("fi", &c, Some((&usage, &id))), &c)[0];
        let other = if first == "Files" { "Firefox" } else { "Files" };
        usage.record(other, "fi");
        assert_eq!(titles(&r.rank("fi", &c, Some((&usage, &id))), &c)[0], other);
    }
}
