//! Rule index for fast selector matching (DROP-120).
//!
//! Every selector is filed under one key taken from its **rightmost** compound selector:
//! its id, else its first class, else its tag, else the universal list. A selector can only
//! match an element whose id / class list / tag contains that key, so for each element only
//! the rules in the element's own buckets need full matching. Results are identical to testing
//! every rule; the cascade still sorts by (specificity, source order).

use std::collections::HashMap;

use droplet_dom::ElementData;

use crate::css::{CompoundSelector, Stylesheet};

const BLOOM_BITS: usize = 4096;
const BLOOM_MASK: u32 = (BLOOM_BITS as u32) - 1;
/// Ancestor keys checked per selector; more adds little and costs per-candidate time.
const MAX_ANCESTOR_HASHES: usize = 4;

const KIND_TAG: u8 = 1;
const KIND_ID: u8 = 2;
const KIND_CLASS: u8 = 3;

/// FNV-1a over a namespace byte plus the key; tags are hashed ASCII-lowercase.
fn key_hash(kind: u8, key: &str) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    let mut feed = |b: u8| {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    };
    feed(kind);
    for b in key.bytes() {
        feed(if kind == KIND_TAG { b.to_ascii_lowercase() } else { b });
    }
    h
}

/// Counting Bloom filter of the id / classes / tag of every ancestor of the element being
/// styled (Servo-style). `might_contain == false` proves no ancestor has that key, so any
/// selector needing it in its descendant/child prefix cannot match. 4 KiB, no per-node memory.
pub(crate) struct AncestorBloom {
    counters: Box<[u8; BLOOM_BITS]>,
}

impl AncestorBloom {
    pub(crate) fn new() -> Self {
        Self {
            counters: Box::new([0; BLOOM_BITS]),
        }
    }

    fn slots(hash: u32) -> (usize, usize) {
        ((hash & BLOOM_MASK) as usize, ((hash >> 12) & BLOOM_MASK) as usize)
    }

    fn insert_hash(&mut self, hash: u32) {
        let (a, b) = Self::slots(hash);
        for slot in [a, b] {
            self.counters[slot] = self.counters[slot].saturating_add(1);
        }
    }

    fn remove_hash(&mut self, hash: u32) {
        let (a, b) = Self::slots(hash);
        for slot in [a, b] {
            // A saturated counter stays sticky: decrementing it could reach 0 while keys
            // remain and produce a false negative (a wrong "no match").
            if self.counters[slot] != u8::MAX {
                self.counters[slot] = self.counters[slot].saturating_sub(1);
            }
        }
    }

    fn might_contain(&self, hash: u32) -> bool {
        let (a, b) = Self::slots(hash);
        self.counters[a] != 0 && self.counters[b] != 0
    }

    fn for_each_key(elem: &ElementData, mut f: impl FnMut(u32)) {
        f(key_hash(KIND_TAG, &elem.tag_name));
        if let Some(id) = elem.id() {
            f(key_hash(KIND_ID, id));
        }
        if let Some(classes) = elem.attr("class") {
            for class in classes.split_whitespace() {
                f(key_hash(KIND_CLASS, class));
            }
        }
    }

    /// Call when descending into `elem`'s children.
    pub(crate) fn push_element(&mut self, elem: &ElementData) {
        Self::for_each_key(elem, |h| self.insert_hash(h));
    }

    /// Call when leaving `elem`'s children (exactly mirrors `push_element`).
    pub(crate) fn pop_element(&mut self, elem: &ElementData) {
        Self::for_each_key(elem, |h| self.remove_hash(h));
    }

    /// `false` only if some required ancestor key is definitely absent.
    pub(crate) fn may_match(&self, hashes: &AncestorHashes) -> bool {
        hashes.hashes[..usize::from(hashes.len)]
            .iter()
            .all(|&h| self.might_contain(h))
    }
}

/// Keys that must appear among an element's ancestors for a selector to match
/// (taken from every compound left of the rightmost one).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct AncestorHashes {
    hashes: [u32; MAX_ANCESTOR_HASHES],
    len: u8,
}

impl AncestorHashes {
    fn from_prefix(prefix: &[(Option<crate::css::Combinator>, CompoundSelector)]) -> Self {
        let mut out = Self::default();
        let mut push = |h: u32| {
            if usize::from(out.len) < MAX_ANCESTOR_HASHES {
                out.hashes[usize::from(out.len)] = h;
                out.len += 1;
            }
        };
        // Nearest compounds first: they are the most likely to reject.
        for (_, compound) in prefix.iter().rev() {
            if let Some(id) = &compound.id {
                push(key_hash(KIND_ID, id));
            }
            for class in &compound.classes {
                push(key_hash(KIND_CLASS, class));
            }
            if let Some(tag) = &compound.tag_name {
                push(key_hash(KIND_TAG, tag));
            }
        }
        out
    }
}

/// Rule indices bucketed by the rightmost compound selector's most selective key.
#[derive(Debug, Default)]
pub(crate) struct RuleIndex {
    by_id: HashMap<String, Vec<u32>>,
    by_class: HashMap<String, Vec<u32>>,
    /// Keys are ASCII-lowercase (tag matching is case-insensitive).
    by_tag: HashMap<String, Vec<u32>>,
    universal: Vec<u32>,
    /// `[rule][selector]` ancestor keys, parallel to `Stylesheet::rules[..].selectors`.
    ancestor_hashes: Vec<Vec<AncestorHashes>>,
}

fn push_unique(bucket: &mut Vec<u32>, idx: u32) {
    // Selectors of one rule are visited consecutively, so a duplicate is always the last entry.
    if bucket.last() != Some(&idx) {
        bucket.push(idx);
    }
}

impl RuleIndex {
    pub(crate) fn build(sheet: &Stylesheet) -> Self {
        let mut index = Self {
            ancestor_hashes: Self::build_ancestor_hashes(sheet),
            ..Self::default()
        };
        for (rule_idx, rule) in sheet.rules.iter().enumerate() {
            let idx = rule_idx as u32;
            for sel in &rule.selectors {
                let Some((_, last)) = sel.parts.last() else {
                    continue;
                };
                // A compound with an unsupported pseudo-class never matches (`matches_compound`),
                // so neither does the whole selector.
                if sel.parts.iter().any(|(_, c)| c.unsupported_pseudo) {
                    continue;
                }
                if let Some(id) = &last.id {
                    push_unique(index.by_id.entry(id.clone()).or_default(), idx);
                } else if let Some(class) = last.classes.first() {
                    push_unique(index.by_class.entry(class.clone()).or_default(), idx);
                } else if let Some(tag) = &last.tag_name {
                    push_unique(
                        index.by_tag.entry(tag.to_ascii_lowercase()).or_default(),
                        idx,
                    );
                } else {
                    push_unique(&mut index.universal, idx);
                }
            }
        }
        index
    }

    fn build_ancestor_hashes(sheet: &Stylesheet) -> Vec<Vec<AncestorHashes>> {
        sheet
            .rules
            .iter()
            .map(|rule| {
                rule.selectors
                    .iter()
                    .map(|sel| {
                        let prefix_len = sel.parts.len().saturating_sub(1);
                        AncestorHashes::from_prefix(&sel.parts[..prefix_len])
                    })
                    .collect()
            })
            .collect()
    }

    /// Ancestor keys for selector `sel` of rule `rule` (empty if out of range).
    pub(crate) fn ancestor_hashes(&self, rule: usize, sel: usize) -> AncestorHashes {
        self.ancestor_hashes
            .get(rule)
            .and_then(|sels| sels.get(sel))
            .copied()
            .unwrap_or_default()
    }

    /// Writes the candidate rule indices for `elem` into `out`, sorted ascending and deduplicated.
    pub(crate) fn candidates(&self, elem: &ElementData, out: &mut Vec<u32>) {
        out.clear();
        if !self.by_id.is_empty() {
            if let Some(bucket) = elem.id().and_then(|id| self.by_id.get(id)) {
                out.extend_from_slice(bucket);
            }
        }
        if !self.by_class.is_empty() {
            if let Some(classes) = elem.attr("class") {
                for class in classes.split_whitespace() {
                    if let Some(bucket) = self.by_class.get(class) {
                        out.extend_from_slice(bucket);
                    }
                }
            }
        }
        if !self.by_tag.is_empty() {
            let tag = elem.tag_name.as_str();
            let bucket = if tag.bytes().any(|b| b.is_ascii_uppercase()) {
                self.by_tag.get(&tag.to_ascii_lowercase())
            } else {
                self.by_tag.get(tag)
            };
            if let Some(bucket) = bucket {
                out.extend_from_slice(bucket);
            }
        }
        out.extend_from_slice(&self.universal);
        out.sort_unstable();
        out.dedup();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::css;

    fn element(tag: &str, attrs: &[(&str, &str)]) -> ElementData {
        ElementData {
            tag_name: tag.to_string(),
            attributes: attrs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    #[test]
    fn buckets_by_rightmost_compound_and_skips_unsupported() {
        let sheet = css::parse(
            "#main { color: red }\
             .кнопка { color: blue }\
             div p.note { color: green }\
             DIV { margin: 0 }\
             * { padding: 0 }\
             a:visited { color: purple }\
             ul > li, .x li { margin: 1px }",
        );
        let index = RuleIndex::build(&sheet);
        assert_eq!(index.by_id.get("main"), Some(&vec![0]));
        assert_eq!(index.by_class.get("кнопка"), Some(&vec![1]));
        // `div p.note`: rightmost compound has a class → class bucket, not tag bucket.
        assert_eq!(index.by_class.get("note"), Some(&vec![2]));
        assert_eq!(index.by_tag.get("div"), Some(&vec![3]));
        assert_eq!(index.universal, vec![4]);
        // `a:visited` is unsupported and can never match.
        assert!(!index.by_tag.contains_key("a"));
        // Both selectors of rule 6 key on `li`, stored once.
        assert_eq!(index.by_tag.get("li"), Some(&vec![6]));
    }

    #[test]
    fn candidates_are_sorted_deduplicated_and_case_insensitive_for_tags() {
        let sheet = css::parse(
            "p { color: red } .a { color: blue } .b { color: green } #x { color: black } * { margin: 0 }",
        );
        let index = RuleIndex::build(&sheet);
        let mut out = Vec::new();
        index.candidates(&element("P", &[("class", "b a a"), ("id", "x")]), &mut out);
        assert_eq!(out, vec![0, 1, 2, 3, 4]);
        index.candidates(&element("span", &[("class", "нет")]), &mut out);
        assert_eq!(out, vec![4]);
    }

    #[test]
    fn bloom_tracks_ancestors_and_never_gives_false_negatives() {
        let sheet = css::parse(".меню #top DIV p { color: red } section > p { color: blue }");
        let index = RuleIndex::build(&sheet);
        let needs_menu_top_div = index.ancestor_hashes(0, 0);
        let needs_section = index.ancestor_hashes(1, 0);
        assert_eq!(needs_menu_top_div.len, 3);

        let mut bloom = AncestorBloom::new();
        assert!(!bloom.may_match(&needs_section));
        let outer = element("div", &[("class", "меню x"), ("id", "top")]);
        bloom.push_element(&outer);
        // `.меню` and `#top` present, tag `div` present (same element): may match.
        assert!(bloom.may_match(&needs_menu_top_div));
        assert!(!bloom.may_match(&needs_section));
        let section = element("SECTION", &[]);
        bloom.push_element(&section);
        assert!(bloom.may_match(&needs_section));
        bloom.pop_element(&section);
        bloom.pop_element(&outer);
        assert!(!bloom.may_match(&needs_menu_top_div));
        assert!(bloom.counters.iter().all(|&c| c == 0));
    }

    #[test]
    fn saturated_bloom_counters_stay_sticky() {
        let mut bloom = AncestorBloom::new();
        let h = key_hash(KIND_CLASS, "a");
        for _ in 0..300 {
            bloom.insert_hash(h);
        }
        for _ in 0..299 {
            bloom.remove_hash(h);
        }
        assert!(bloom.might_contain(h), "one insert remains: must still report present");
    }
}
