//! Translations, with Fluent (Project Fluent, by Mozilla).
//!
//! The messages are in `data/i18n/<locale>.ftl`, embedded in the binaries.
//! English (`en-US`) is the source and the fallback: a message missing from
//! the user's language is shown in English, and a message missing from both
//! shows its id. To add a language, add its `.ftl` file to `data/i18n/` and
//! to `CATALOGS` below.
//!
//! The language comes from the environment, as gettext would pick it:
//! `LC_ALL`, then `LC_MESSAGES`, then `LANG`, then `LANGUAGE`'s first entry.
//! An exact match wins (`pt_BR` -> pt-BR), then one of the same language
//! (`pt_PT` -> pt-BR), then English.

use std::cell::OnceCell;

use fluent_bundle::{FluentArgs, FluentBundle, FluentResource, FluentValue};
use unic_langid::LanguageIdentifier;

/// (locale, catalog), English first.
const CATALOGS: &[(&str, &str)] = &[
    ("en-US", include_str!("../../../data/i18n/en-US.ftl")),
    ("pt-BR", include_str!("../../../data/i18n/pt-BR.ftl")),
];

struct Catalogs {
    /// The user's language, when it is not English.
    user: Option<FluentBundle<FluentResource>>,
    english: FluentBundle<FluentResource>,
}

thread_local! {
    static CATALOGS_CELL: OnceCell<Catalogs> = const { OnceCell::new() };
}

fn bundle(locale: &str, source: &str) -> FluentBundle<FluentResource> {
    let id: LanguageIdentifier = locale.parse().expect("valid locale in CATALOGS");
    let mut bundle = FluentBundle::new(vec![id]);
    // No Unicode isolation marks around arguments: GTK labels would carry
    // them as invisible characters, and widths in the monospace panel font
    // would no longer add up.
    bundle.set_use_isolating(false);
    let resource = FluentResource::try_new(source.to_string()).unwrap_or_else(|(res, errors)| {
        log::error!("i18n: errors in the {locale} catalog: {errors:?}");
        res
    });
    if let Err(errors) = bundle.add_resource(resource) {
        log::error!("i18n: duplicate messages in the {locale} catalog: {errors:?}");
    }
    bundle
}

/// The locale requested by the environment, as `ll-CC` (`pt-BR`).
fn requested() -> Option<String> {
    let value = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|v| std::env::var(v).ok())
        .find(|v| !v.is_empty())
        .or_else(|| {
            std::env::var("LANGUAGE")
                .ok()
                .and_then(|v| v.split(':').next().map(str::to_owned))
        })?;
    // pt_BR.UTF-8@euro -> pt-BR
    let base = value.split(['.', '@']).next()?.replace('_', "-");
    (base != "C" && base != "POSIX" && !base.is_empty()).then_some(base)
}

/// The catalog for `wanted`: exact match, else same language, else none.
fn choose(wanted: &str) -> Option<&'static (&'static str, &'static str)> {
    let language = |l: &str| l.split('-').next().unwrap_or_default().to_ascii_lowercase();
    CATALOGS
        .iter()
        .find(|(locale, _)| locale.eq_ignore_ascii_case(wanted))
        .or_else(|| {
            CATALOGS
                .iter()
                .find(|(locale, _)| language(locale) == language(wanted))
        })
}

fn with<T>(f: impl FnOnce(&Catalogs) -> T) -> T {
    CATALOGS_CELL.with(|cell| {
        f(cell.get_or_init(|| {
            let (english_locale, english_source) = CATALOGS[0];
            let user = requested()
                .and_then(|wanted| choose(&wanted))
                .filter(|(locale, _)| *locale != english_locale)
                .map(|(locale, source)| bundle(locale, source));
            Catalogs {
                user,
                english: bundle(english_locale, english_source),
            }
        }))
    })
}

fn format(
    bundle: &FluentBundle<FluentResource>,
    id: &str,
    args: Option<&FluentArgs>,
) -> Option<String> {
    let pattern = bundle.get_message(id)?.value()?;
    let mut errors = Vec::new();
    let text = bundle
        .format_pattern(pattern, args, &mut errors)
        .into_owned();
    if !errors.is_empty() {
        log::warn!("i18n: {id}: {errors:?}");
    }
    Some(text)
}

/// The message `id` in the user's language.
pub fn tr(id: &str) -> String {
    tr_args(id, &[])
}

/// The message `id` with its `{ $name }` placeholders filled from `args`.
pub fn tr_args(id: &str, args: &[(&str, FluentValue)]) -> String {
    let fluent_args = (!args.is_empty()).then(|| {
        let mut a = FluentArgs::new();
        for (name, value) in args {
            a.set(*name, value.clone());
        }
        a
    });
    with(|c| {
        c.user
            .as_ref()
            .and_then(|b| format(b, id, fluent_args.as_ref()))
            .or_else(|| format(&c.english, id, fluent_args.as_ref()))
            .unwrap_or_else(|| id.to_string())
    })
}

/// Shorthand for a text argument.
pub fn text(value: impl Into<String>) -> FluentValue<'static> {
    FluentValue::from(value.into())
}

/// Shorthand for a number argument.
pub fn number(value: impl Into<f64>) -> FluentValue<'static> {
    FluentValue::from(value.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_catalog_parses_and_has_every_english_message() {
        let english = FluentResource::try_new(CATALOGS[0].1.to_string()).expect("en-US parses");
        let ids: Vec<String> = english
            .entries()
            .filter_map(|e| match e {
                fluent_syntax::ast::Entry::Message(m) => Some(m.id.name.to_string()),
                _ => None,
            })
            .collect();
        assert!(!ids.is_empty());
        for (locale, source) in &CATALOGS[1..] {
            let res = FluentResource::try_new(source.to_string())
                .unwrap_or_else(|(_, e)| panic!("{locale} does not parse: {e:?}"));
            let has: Vec<String> = res
                .entries()
                .filter_map(|e| match e {
                    fluent_syntax::ast::Entry::Message(m) => Some(m.id.name.to_string()),
                    _ => None,
                })
                .collect();
            let missing: Vec<&String> = ids.iter().filter(|id| !has.contains(id)).collect();
            assert!(missing.is_empty(), "{locale} lacks {missing:?}");
        }
    }

    /// Formats `id` straight from the catalog of `locale`.
    fn in_locale(locale: &str, id: &str, args: &[(&str, FluentValue)]) -> String {
        let (_, source) = choose(locale).expect("catalog");
        let mut a = FluentArgs::new();
        for (name, value) in args {
            a.set(*name, value.clone());
        }
        format(&bundle(locale, source), id, Some(&a)).expect("message")
    }

    #[test]
    fn messages_in_both_languages() {
        assert_eq!(in_locale("en-US", "weather-code-3", &[]), "Overcast");
        assert_eq!(in_locale("pt-BR", "weather-code-3", &[]), "Nublado");
        let muted = |locale, m| {
            in_locale(
                locale,
                "volume-tooltip",
                &[("percent", number(41.0)), ("muted", text(m))],
            )
        };
        assert!(
            muted("en-US", "yes").starts_with("volume: 41% (muted) — click"),
            "{}",
            muted("en-US", "yes")
        );
        assert!(
            muted("pt-BR", "no").starts_with("volume: 41% — clique"),
            "{}",
            muted("pt-BR", "no")
        );
        assert_eq!(
            in_locale("pt-BR", "windows-area", &[("area", number(3.0))]),
            "Área 3"
        );
        for locale in ["en-US", "pt-BR"] {
            assert_eq!(
                in_locale(locale, "weather-compass", &[])
                    .split_whitespace()
                    .count(),
                16
            );
        }
        assert!(in_locale("en-US", "weather-compass", &[]).contains(" E "));
        assert!(in_locale("pt-BR", "weather-compass", &[]).contains(" L "));
    }

    #[test]
    fn locale_choice() {
        assert_eq!(choose("pt-BR").map(|c| c.0), Some("pt-BR"));
        assert_eq!(choose("pt-PT").map(|c| c.0), Some("pt-BR"), "same language");
        assert_eq!(choose("en-GB").map(|c| c.0), Some("en-US"));
        assert_eq!(choose("ja-JP"), None, "falls back to English");
    }
}
