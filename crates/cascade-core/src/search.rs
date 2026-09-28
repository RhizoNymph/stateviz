//! Fuzzy search over every named element.

use crate::key::{ElementKind, ElementRef};
use crate::model::Model;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchHit {
    pub element: ElementRef,
    pub label: String,
    /// Higher is better.
    pub score: i64,
    /// Char indices into `label` that matched the query, for highlighting.
    pub matched: Vec<usize>,
}

/// Elements whose label contains the query's characters in order
/// (case-insensitive), best first, at most `limit` hits. Contiguous runs,
/// word starts and short labels score higher; ties keep model order.
pub fn search(model: &Model, query: &str, limit: usize) -> Vec<SearchHit> {
    let query: Vec<char> = query.chars().filter(|c| !c.is_whitespace()).flat_map(char::to_lowercase).collect();
    if query.is_empty() || limit == 0 {
        return Vec::new();
    }
    let mut hits: Vec<(usize, SearchHit)> = model
        .all_elements()
        .into_iter()
        // Triggers, handlers and rules are reachable through their
        // transitions and controllers; listing them too would crowd results.
        .filter(|e| !matches!(e.kind(), ElementKind::Trigger | ElementKind::Handler | ElementKind::Rule))
        .enumerate()
        .filter_map(|(order, element)| {
            let label = model.label_of(element);
            score(&label, &query).map(|(score, matched)| (order, SearchHit { element, label, score, matched }))
        })
        .collect();
    hits.sort_by(|(ao, a), (bo, b)| b.score.cmp(&a.score).then(ao.cmp(bo)));
    hits.into_iter().take(limit).map(|(_, hit)| hit).collect()
}

/// Greedy subsequence match with bonuses; `None` when not all query chars
/// appear in order.
fn score(label: &str, query: &[char]) -> Option<(i64, Vec<usize>)> {
    let chars: Vec<char> = label.chars().collect();
    let lower: Vec<char> = chars.iter().map(|c| c.to_lowercase().next().unwrap_or(*c)).collect();
    let mut matched = Vec::with_capacity(query.len());
    let mut score: i64 = 0;
    let mut qi = 0;
    let mut prev: Option<usize> = None;
    for (i, &c) in lower.iter().enumerate() {
        if qi == query.len() {
            break;
        }
        if c != query[qi] {
            continue;
        }
        score += 1;
        let at_word_start = i == 0 || {
            let before = chars[i - 1];
            !before.is_alphanumeric() || (before.is_lowercase() && chars[i].is_uppercase())
        };
        if at_word_start {
            score += 8;
        }
        if prev.is_some_and(|p| p + 1 == i) {
            score += 5;
        }
        matched.push(i);
        prev = Some(i);
        qi += 1;
    }
    if qi < query.len() {
        return None;
    }
    let len_penalty = i64::try_from(chars.len()).unwrap_or(i64::MAX) / 4;
    Some((score * 4 - len_penalty, matched))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scores_prefer_contiguous_word_starts() {
        let (paid, _) = score("Order: pending → paid", &['p', 'a', 'i', 'd']).expect("match");
        let (spread, _) = score("pxaxixd", &['p', 'a', 'i', 'd']).expect("match");
        assert!(paid > spread);
    }

    #[test]
    fn no_match_when_out_of_order() {
        assert_eq!(score("abc", &['c', 'a']), None);
    }

    #[test]
    fn matched_indices_point_into_label() {
        let (_, matched) = score("OrderPaid", &['o', 'p']).expect("match");
        assert_eq!(matched, vec![0, 5]);
    }
}
