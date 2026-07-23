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
    let value = value.trim();
    let mut subtags = value.split(['_', '-']);
    let language = subtags.next()?;
    if !(2..=8).contains(&language.len())
        || !language
            .chars()
            .all(|character| character.is_ascii_alphabetic())
    {
        return None;
    }

    let mut normalized = language.to_ascii_lowercase();
    for subtag in subtags {
        if subtag.is_empty()
            || subtag.len() > 8
            || !subtag
                .chars()
                .all(|character| character.is_ascii_alphanumeric())
        {
            return None;
        }

        normalized.push('-');
        if is_region(subtag) {
            normalized.push_str(&subtag.to_ascii_uppercase());
        } else {
            normalized.push_str(subtag);
        }
    }
    Some(normalized)
}

fn is_region(subtag: &str) -> bool {
    (subtag.len() == 2
        && subtag
            .chars()
            .all(|character| character.is_ascii_alphabetic()))
        || (subtag.len() == 3 && subtag.chars().all(|character| character.is_ascii_digit()))
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
    fn locale_normalization_preserves_script_and_variant() {
        assert_eq!(
            normalize_locale("ZH_Hant_cn_fonipa"),
            Some("zh-Hant-CN-fonipa".into())
        );
    }
}
