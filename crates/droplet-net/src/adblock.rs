use std::cmp::Ordering;

const DEFAULT_TRACKER_DOMAINS: &[&str] = &[
    "2mdn.net",
    "adcolony.com",
    "adjust.com",
    "adnxs.com",
    "adroll.com",
    "adservice.google.com",
    "adsrvr.org",
    "advertising.com",
    "amazon-adsystem.com",
    "amplitude.com",
    "an.yandex.ru",
    "app-measurement.com",
    "appsflyer.com",
    "bam.nr-data.net",
    "branch.io",
    "bugsnag.com",
    "chartbeat.com",
    "clarity.ms",
    "connect.facebook.net",
    "criteo.com",
    "criteo.net",
    "demdex.net",
    "doubleclick.net",
    "facebook.net",
    "fullstory.com",
    "google-analytics.com",
    "googleadservices.com",
    "googlesyndication.com",
    "googletagmanager.com",
    "googletagservices.com",
    "heapanalytics.com",
    "hotjar.com",
    "hs-analytics.net",
    "hs-scripts.com",
    "krxd.net",
    "logrocket.io",
    "mc.yandex.ru",
    "mixpanel.com",
    "moatads.com",
    "omtrdc.net",
    "openx.net",
    "outbrain.com",
    "pendo.io",
    "pixel.facebook.com",
    "pubmatic.com",
    "quantserve.com",
    "rubiconproject.com",
    "scorecardresearch.com",
    "segment.com",
    "segment.io",
    "sentry.io",
    "taboola.com",
    "tapad.com",
    "telemetry.mozilla.org",
    "top-fwz1.mail.ru",
    "yandexadexchange.net",
];

#[derive(Debug, Clone)]
pub struct TrackerFilter {
    defaults: &'static [&'static str],
    custom: Vec<String>,
}

impl Default for TrackerFilter {
    fn default() -> Self {
        Self::with_defaults()
    }
}

impl TrackerFilter {
    pub fn with_defaults() -> Self {
        Self {
            defaults: DEFAULT_TRACKER_DOMAINS,
            custom: Vec::new(),
        }
    }

    pub fn empty() -> Self {
        Self {
            defaults: &[],
            custom: Vec::new(),
        }
    }

    pub fn add_domain(&mut self, domain: &str) {
        let normalized = domain.trim().trim_matches('.').to_ascii_lowercase();
        if normalized.is_empty() {
            return;
        }
        if let Err(idx) = self.custom.binary_search(&normalized) {
            self.custom.insert(idx, normalized);
        }
    }

    pub fn should_block(&self, host: &str) -> bool {
        let trimmed = host.trim().trim_end_matches('.');
        if trimmed.is_empty() || trimmed.starts_with('[') {
            return false;
        }

        let mut candidate = trimmed.trim_start_matches('.');
        while !candidate.is_empty() {
            if self.matches_exact(candidate) {
                return true;
            }
            match candidate.split_once('.') {
                Some((_, rest)) => candidate = rest.trim_start_matches('.'),
                None => break,
            }
        }
        false
    }

    fn matches_exact(&self, candidate: &str) -> bool {
        if !self.defaults.is_empty()
            && self
                .defaults
                .binary_search_by(|entry| cmp_ascii_ignore_case(entry, candidate))
                .is_ok()
        {
            return true;
        }
        !self.custom.is_empty()
            && self
                .custom
                .binary_search_by(|entry| cmp_ascii_ignore_case(entry, candidate))
                .is_ok()
    }
}

fn cmp_ascii_ignore_case(a: &str, b: &str) -> Ordering {
    for (&ba, &bb) in a.as_bytes().iter().zip(b.as_bytes().iter()) {
        let ca = ba.to_ascii_lowercase();
        let cb = bb.to_ascii_lowercase();
        match ca.cmp(&cb) {
            Ordering::Equal => continue,
            non_eq => return non_eq,
        }
    }
    a.len().cmp(&b.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_domains_are_sorted_and_normalized() {
        assert!(!DEFAULT_TRACKER_DOMAINS.is_empty());
        for domain in DEFAULT_TRACKER_DOMAINS {
            assert!(!domain.is_empty());
            assert_eq!(*domain, domain.to_ascii_lowercase());
            assert!(!domain.starts_with('.'));
            assert!(!domain.ends_with('.'));
        }
        for pair in DEFAULT_TRACKER_DOMAINS.windows(2) {
            assert!(
                pair[0] < pair[1],
                "DEFAULT_TRACKER_DOMAINS out of order: '{}' >= '{}'",
                pair[0],
                pair[1]
            );
        }
    }

    #[test]
    fn blocks_exact_and_subdomain_matches() {
        let filter = TrackerFilter::with_defaults();
        assert!(filter.should_block("doubleclick.net"));
        assert!(filter.should_block("ad.doubleclick.net"));
        assert!(filter.should_block("stats.g.doubleclick.net"));
        assert!(filter.should_block("google-analytics.com"));
        assert!(filter.should_block("www.googletagmanager.com"));
        assert!(filter.should_block("mc.yandex.ru"));
        assert!(filter.should_block("top-fwz1.mail.ru"));
        assert!(filter.should_block("sentry.io"));
        assert!(filter.should_block("o123.ingest.sentry.io"));
    }

    #[test]
    fn case_insensitive_and_trailing_dot() {
        let filter = TrackerFilter::with_defaults();
        assert!(filter.should_block("DOUBLECLICK.NET"));
        assert!(filter.should_block("PageAd2.GoogleSyndication.COM."));
        assert!(filter.should_block("  MC.Yandex.RU.. "));
    }

    #[test]
    fn allows_negative_boundary_and_legit_hosts() {
        let filter = TrackerFilter::with_defaults();
        assert!(!filter.should_block("notdoubleclick.net"));
        assert!(!filter.should_block("mygooglesyndication.com"));
        assert!(!filter.should_block("example.com"));
        assert!(!filter.should_block("yandex.ru"));
        assert!(!filter.should_block("mail.ru"));
        assert!(!filter.should_block("google.com"));
        assert!(!filter.should_block("[::1]"));
        assert!(!filter.should_block(""));
        assert!(!filter.should_block("."));
    }

    #[test]
    fn empty_filter_and_custom_domains() {
        let mut filter = TrackerFilter::empty();
        assert!(!filter.should_block("doubleclick.net"));

        filter.add_domain("  .Tracker.Example. ");
        filter.add_domain("tracker.example");
        filter.add_domain("");

        assert!(filter.should_block("tracker.example"));
        assert!(filter.should_block("ads.tracker.example"));
        assert!(filter.should_block("ADS.TRACKER.EXAMPLE."));
        assert!(!filter.should_block("mytracker.example"));
        assert!(!filter.should_block("example"));
    }
}
