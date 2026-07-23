use jiff::tz::TimeZone;
use wokrouter_storage::UiConfig;

const FALLBACK_LOCALE: &str = "en";
const FALLBACK_TIMEZONE: &str = "UTC";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SystemContext {
    pub locale: String,
    pub timezone: String,
}

pub fn detect_system_context(ui: &UiConfig) -> SystemContext {
    let system_locale = sys_locale::get_locale();
    let system_timezone = system_timezone();

    context_from_candidates(ui, system_locale.as_deref(), system_timezone.as_deref())
}

fn context_from_candidates(
    ui: &UiConfig,
    system_locale: Option<&str>,
    system_timezone: Option<&str>,
) -> SystemContext {
    let locale = ui
        .locale_override
        .as_deref()
        .and_then(normalize_locale)
        .or_else(|| system_locale.and_then(normalize_locale))
        .unwrap_or_else(|| FALLBACK_LOCALE.into());
    let timezone = ui
        .timezone_override
        .as_deref()
        .and_then(normalize_timezone)
        .or_else(|| system_timezone.and_then(normalize_timezone))
        .unwrap_or_else(|| FALLBACK_TIMEZONE.into());

    SystemContext { locale, timezone }
}

fn normalize_locale(value: &str) -> Option<String> {
    let subtags: Vec<_> = value.trim().split(['_', '-']).collect();
    if subtags.is_empty() || subtags.iter().any(|subtag| subtag.is_empty()) {
        return None;
    }

    if subtags[0].eq_ignore_ascii_case("x") {
        if subtags.len() == 1 || !subtags[1..].iter().all(|subtag| is_private_use(subtag)) {
            return None;
        }
        return Some(join_subtags("x", &subtags[1..]));
    }

    let language = subtags[0];
    if !is_language(language) {
        return None;
    }

    let mut normalized = language.to_ascii_lowercase();
    let mut index = 1;
    if subtags.get(index).is_some_and(|subtag| is_script(subtag)) {
        normalized.push('-');
        normalized.push_str(&titlecase_ascii(subtags[index]));
        index += 1;
    }
    if subtags.get(index).is_some_and(|subtag| is_region(subtag)) {
        normalized.push('-');
        normalized.push_str(&subtags[index].to_ascii_uppercase());
        index += 1;
    }
    while subtags.get(index).is_some_and(|subtag| is_variant(subtag)) {
        normalized.push('-');
        normalized.push_str(subtags[index]);
        index += 1;
    }

    let mut extension_singletons = Vec::new();
    while let Some(subtag) = subtags.get(index) {
        if subtag.eq_ignore_ascii_case("x") {
            let private_use = &subtags[index + 1..];
            if private_use.is_empty() || !private_use.iter().all(|part| is_private_use(part)) {
                return None;
            }
            normalized.push_str("-x");
            for part in private_use {
                normalized.push('-');
                normalized.push_str(part);
            }
            return Some(normalized);
        }
        if !is_extension_singleton(subtag) {
            return None;
        }

        let singleton = subtag.to_ascii_lowercase();
        if extension_singletons.contains(&singleton) {
            return None;
        }
        normalized.push('-');
        normalized.push_str(&singleton);
        extension_singletons.push(singleton);
        index += 1;

        let payload_start = index;
        while subtags
            .get(index)
            .is_some_and(|part| is_extension_payload(part))
        {
            normalized.push('-');
            normalized.push_str(subtags[index]);
            index += 1;
        }
        if index == payload_start {
            return None;
        }
    }
    Some(normalized)
}

fn is_language(subtag: &str) -> bool {
    (2..=8).contains(&subtag.len())
        && subtag
            .chars()
            .all(|character| character.is_ascii_alphabetic())
}

fn is_script(subtag: &str) -> bool {
    subtag.len() == 4
        && subtag
            .chars()
            .all(|character| character.is_ascii_alphabetic())
}

fn is_region(subtag: &str) -> bool {
    (subtag.len() == 2
        && subtag
            .chars()
            .all(|character| character.is_ascii_alphabetic()))
        || (subtag.len() == 3 && subtag.chars().all(|character| character.is_ascii_digit()))
}

fn is_variant(subtag: &str) -> bool {
    ((5..=8).contains(&subtag.len())
        && subtag
            .chars()
            .all(|character| character.is_ascii_alphanumeric()))
        || (subtag.len() == 4
            && subtag.starts_with(|character: char| character.is_ascii_digit())
            && subtag
                .chars()
                .all(|character| character.is_ascii_alphanumeric()))
}

fn is_extension_singleton(subtag: &str) -> bool {
    subtag.len() == 1
        && !subtag.eq_ignore_ascii_case("x")
        && subtag
            .chars()
            .all(|character| character.is_ascii_alphanumeric())
}

fn is_extension_payload(subtag: &str) -> bool {
    (2..=8).contains(&subtag.len())
        && subtag
            .chars()
            .all(|character| character.is_ascii_alphanumeric())
}

fn is_private_use(subtag: &str) -> bool {
    (1..=8).contains(&subtag.len())
        && subtag
            .chars()
            .all(|character| character.is_ascii_alphanumeric())
}

fn join_subtags(first: &str, subtags: &[&str]) -> String {
    let mut normalized = first.to_owned();
    for subtag in subtags {
        normalized.push('-');
        normalized.push_str(subtag);
    }
    normalized
}

fn titlecase_ascii(subtag: &str) -> String {
    let mut normalized = subtag.to_ascii_lowercase();
    normalized[..1].make_ascii_uppercase();
    normalized
}

fn normalize_timezone(value: &str) -> Option<String> {
    TimeZone::get(value.trim())
        .ok()?
        .iana_name()
        .map(str::to_owned)
}

fn system_timezone() -> Option<String> {
    TimeZone::system().iana_name().map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_system_values_fall_back_to_english_and_utc() {
        let context = context_from_candidates(
            &UiConfig {
                locale_override: None,
                timezone_override: None,
            },
            None,
            None,
        );

        assert_eq!(context.locale, "en");
        assert_eq!(context.timezone, "UTC");
    }

    #[test]
    fn locale_normalization_canonicalizes_language_script_and_region() {
        assert_eq!(normalize_locale("zh_hans_cn"), Some("zh-Hans-CN".into()));
        assert_eq!(
            normalize_locale("ZH_Hant_cn_fonipa"),
            Some("zh-Hant-CN-fonipa".into())
        );
    }

    #[test]
    fn pure_private_use_locale_is_accepted() {
        assert_eq!(normalize_locale("x-private"), Some("x-private".into()));
    }

    #[test]
    fn extension_payload_is_not_cased_as_a_region() {
        assert_eq!(
            normalize_locale("en-u-ca-gregory"),
            Some("en-u-ca-gregory".into())
        );
    }

    #[test]
    fn duplicate_extension_singleton_falls_back_to_the_next_locale_candidate() {
        let context = context_from_candidates(
            &UiConfig {
                locale_override: Some("en-u-ca-u-nu-latn".into()),
                timezone_override: None,
            },
            Some("fr_FR"),
            None,
        );

        assert_eq!(context.locale, "fr-FR");
    }

    #[test]
    fn duplicate_extension_singletons_are_case_insensitive_after_separator_normalization() {
        assert_eq!(normalize_locale("en_U_ca_u_NU_latn"), None);
    }

    #[test]
    fn distinct_extension_singletons_with_payloads_are_valid() {
        assert_eq!(
            normalize_locale("en-a-foo-b-bar"),
            Some("en-a-foo-b-bar".into())
        );
    }

    #[test]
    fn private_use_payload_is_not_cased_as_a_region() {
        assert_eq!(normalize_locale("en-x-us"), Some("en-x-us".into()));
    }

    #[test]
    fn empty_subtags_fall_back_to_the_next_locale_candidate() {
        let context = context_from_candidates(
            &UiConfig {
                locale_override: Some("en--US".into()),
                timezone_override: None,
            },
            Some("pt_BR"),
            None,
        );

        assert_eq!(context.locale, "pt-BR");
        assert_eq!(normalize_locale("-en"), None);
        assert_eq!(normalize_locale("en-"), None);

        for locale_override in ["-en", "en-"] {
            let context = context_from_candidates(
                &UiConfig {
                    locale_override: Some(locale_override.into()),
                    timezone_override: None,
                },
                None,
                None,
            );
            assert_eq!(context.locale, "en");
        }
    }
}
