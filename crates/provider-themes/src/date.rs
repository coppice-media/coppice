//! Chapter date parsing for definition-driven engines.
//!
//! Sites publish chapter dates in three shapes and every theme handles all
//! three:
//!
//! 1. an absolute date matched against the theme's `SimpleDateFormat` /
//!    `DateTimeFormatter` pattern (`date_format` + `date_locale`);
//! 2. an ISO-8601 `datetime` attribute;
//! 3. a relative phrase — "2 days ago", "hace 3 horas", "5 dakika önce".
//!
//! The relative vocabulary and the "today"/"yesterday" prefixes are the ones
//! `lib-multisrc/madara/.../MadaraBase.kt` carries in its companion object
//! (`YEAR_WORDS` … `SECOND_WORDS`), because that is the widest list upstream
//! has and it costs nothing to apply it to the other themes.
//!
//! Month names come from a locale table rather than from `chrono`, which only
//! knows English: `date_locale` is authoritative, and parsing a Turkish or
//! Indonesian month with English names would silently produce wrong dates.

use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveDateTime, TimeZone, Utc};

/// Translate a Java date pattern into a `chrono` `strftime` format.
///
/// Only the fields the three themes' patterns use are translated; anything
/// else is passed through as a literal, which makes the parse fail loudly
/// (returning `None`) rather than matching the wrong field.
pub fn java_pattern_to_strftime(pattern: &str) -> String {
	let mut out = String::with_capacity(pattern.len() + 4);
	let bytes: Vec<char> = pattern.chars().collect();
	let mut index = 0;
	while index < bytes.len() {
		let character = bytes[index];
		if character == '\'' {
			// Java quotes literal text with single quotes; '' is a literal quote.
			index += 1;
			while index < bytes.len() && bytes[index] != '\'' {
				push_literal(&mut out, bytes[index]);
				index += 1;
			}
			index += 1;
			continue;
		}
		if !character.is_ascii_alphabetic() {
			push_literal(&mut out, character);
			index += 1;
			continue;
		}
		let mut run = 1;
		while index + run < bytes.len() && bytes[index + run] == character {
			run += 1;
		}
		match (character, run) {
			('y' | 'u', 2) => out.push_str("%y"),
			('y' | 'u', _) => out.push_str("%Y"),
			('M' | 'L', 1 | 2) => out.push_str("%m"),
			('M' | 'L', 3) => out.push_str("%b"),
			('M' | 'L', _) => out.push_str("%B"),
			('d', _) => out.push_str("%d"),
			('D', _) => out.push_str("%j"),
			('E', 1..=3) => out.push_str("%a"),
			('E', _) => out.push_str("%A"),
			('H' | 'k', _) => out.push_str("%H"),
			('h' | 'K', _) => out.push_str("%I"),
			('m', _) => out.push_str("%M"),
			('s', _) => out.push_str("%S"),
			('S', _) => out.push_str("%f"),
			('a', _) => out.push_str("%p"),
			('z' | 'Z' | 'X', _) => out.push_str("%z"),
			_ => {
				for _ in 0..run {
					push_literal(&mut out, character);
				}
			},
		}
		index += run;
	}
	out
}

fn push_literal(out: &mut String, character: char) {
	if character == '%' {
		out.push_str("%%");
	} else {
		out.push(character);
	}
}

/// A parsed date pattern with the locale its month names belong to.
#[derive(Debug, Clone)]
pub struct DateParser {
	format: String,
	/// BCP-47 primary language subtag, lowercase; empty means English.
	language: String,
}

impl Default for DateParser {
	/// The default shared by all three base classes: `MMMM dd, yyyy`, English.
	fn default() -> Self {
		Self::new("MMMM dd, yyyy", "en-US")
	}
}

impl DateParser {
	pub fn new(pattern: &str, locale: &str) -> Self {
		Self {
			format: java_pattern_to_strftime(pattern),
			language: locale
				.split(['-', '_'])
				.next()
				.unwrap_or_default()
				.to_ascii_lowercase(),
		}
	}

	pub fn format(&self) -> &str {
		&self.format
	}

	/// Parse one chapter date string, trying the pattern, ISO-8601, and the
	/// relative vocabulary in that order. `None` when nothing matches, which
	/// the engines surface as "no upload date" exactly as Mihon's `0L` does.
	pub fn parse(&self, value: &str) -> Option<DateTime<Utc>> {
		let value = value.trim();
		if value.is_empty() {
			return None;
		}
		self.parse_absolute(value)
			.or_else(|| parse_iso(value))
			.or_else(|| parse_relative(value, Utc::now()))
	}

	fn parse_absolute(&self, value: &str) -> Option<DateTime<Utc>> {
		let normalised = self.translate_month_names(value);
		if let Ok(datetime) = NaiveDateTime::parse_from_str(&normalised, &self.format) {
			return Some(Utc.from_utc_datetime(&datetime));
		}
		NaiveDate::parse_from_str(&normalised, &self.format)
			.ok()
			.and_then(midnight)
	}

	/// Rewrite localised month names to English so `%b`/`%B` can match.
	fn translate_month_names(&self, value: &str) -> String {
		if self.language.is_empty() || self.language == "en" {
			return value.to_string();
		}
		let Some(months) = months_for(&self.language) else {
			return value.to_string();
		};
		let lower = value.to_lowercase();
		for (index, names) in months.iter().enumerate() {
			for name in names.iter().filter(|name| !name.is_empty()) {
				if let Some(position) = lower.find(name) {
					let mut out = String::with_capacity(value.len() + 8);
					out.push_str(&value[..position]);
					out.push_str(ENGLISH_MONTHS[index]);
					out.push_str(&value[position + name.len()..]);
					return out;
				}
			}
		}
		value.to_string()
	}
}

/// `<time datetime="...">` and MangaThemesia's "Updated On" value.
pub fn parse_iso(value: &str) -> Option<DateTime<Utc>> {
	let value = value.trim();
	if let Ok(datetime) = DateTime::parse_from_rfc3339(value) {
		return Some(datetime.with_timezone(&Utc));
	}
	if let Ok(date) = NaiveDate::parse_from_str(value, "%Y-%m-%d") {
		return midnight(date);
	}
	NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S")
		.ok()
		.map(|datetime| Utc.from_utc_datetime(&datetime))
}

/// Midnight UTC on `date`.
fn midnight(date: NaiveDate) -> Option<DateTime<Utc>> {
	date.and_hms_opt(0, 0, 0)
		.map(|datetime| Utc.from_utc_datetime(&datetime))
}

/// "3 days ago", "hace 2 semanas", "5 dakika önce", "today", "ontem".
///
/// `now` is a parameter so the behaviour is testable without freezing time.
pub fn parse_relative(value: &str, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
	let lower = value.trim().to_lowercase();
	if lower.is_empty() {
		return None;
	}
	if TODAY_WORDS.iter().any(|word| lower.starts_with(word)) {
		return midnight(now.date_naive());
	}
	if YESTERDAY_WORDS.iter().any(|word| lower.starts_with(word)) {
		return midnight(now.date_naive().pred_opt()?);
	}
	let amount: i64 = lower
		.split(|c: char| !c.is_ascii_digit())
		.find(|part| !part.is_empty())
		.and_then(|digits| digits.parse().ok())?;
	let contains = |words: &[&str]| words.iter().any(|word| mentions(&lower, word));
	if contains(&YEAR_WORDS) {
		// Calendar years, matching ChronoUnit.YEARS.
		let target = now.with_year(now.year() - amount as i32)?;
		return Some(target);
	}
	if contains(&MONTH_WORDS) {
		return Some(minus_months(now, amount as u32));
	}
	if contains(&WEEK_WORDS) {
		return Some(now - Duration::weeks(amount));
	}
	if contains(&DAY_WORDS) {
		return Some(now - Duration::days(amount));
	}
	if contains(&HOUR_WORDS) {
		return Some(now - Duration::hours(amount));
	}
	if contains(&MINUTE_WORDS) {
		return Some(now - Duration::minutes(amount));
	}
	if contains(&SECOND_WORDS) {
		return Some(now - Duration::seconds(amount));
	}
	None
}

/// Whether `phrase` names the unit `word`.
///
/// An ASCII unit must start a word, because the vocabulary MadaraBase carries
/// has short entries that are substrings of unrelated words — Turkish `ay`
/// ("month") sits inside English "days", which would turn "2 days ago" into
/// two months. Non-ASCII units match anywhere, since CJK phrases such as
/// `3天前` are not whitespace separated.
fn mentions(phrase: &str, word: &str) -> bool {
	if !word.is_ascii() {
		return phrase.contains(word);
	}
	phrase
		.split(|c: char| !c.is_alphanumeric())
		.any(|token| token.starts_with(word))
}

fn minus_months(from: DateTime<Utc>, months: u32) -> DateTime<Utc> {
	let total = from.year() * 12 + from.month0() as i32 - months as i32;
	let year = total.div_euclid(12);
	let month = total.rem_euclid(12) as u32 + 1;
	let day = from.day();
	let mut candidate = None;
	for attempt in (1..=day).rev() {
		if let Some(date) = NaiveDate::from_ymd_opt(year, month, attempt) {
			candidate = Some(date);
			break;
		}
	}
	candidate
		.map(|date| date.and_time(from.time()).and_utc())
		.unwrap_or(from)
}

const ENGLISH_MONTHS: [&str; 12] = [
	"January",
	"February",
	"March",
	"April",
	"May",
	"June",
	"July",
	"August",
	"September",
	"October",
	"November",
	"December",
];

/// Lowercase month names per locale, longest form first so the full name is
/// preferred over the abbreviation. Only the languages the three themes' hosts
/// actually declare are carried.
fn months_for(language: &str) -> Option<&'static [[&'static str; 2]; 12]> {
	match language {
		"es" => Some(&SPANISH_MONTHS),
		"pt" => Some(&PORTUGUESE_MONTHS),
		"fr" => Some(&FRENCH_MONTHS),
		"id" | "in" => Some(&INDONESIAN_MONTHS),
		"tr" => Some(&TURKISH_MONTHS),
		"it" => Some(&ITALIAN_MONTHS),
		_ => None,
	}
}

const SPANISH_MONTHS: [[&str; 2]; 12] = [
	["enero", "ene"],
	["febrero", "feb"],
	["marzo", "mar"],
	["abril", "abr"],
	["mayo", "may"],
	["junio", "jun"],
	["julio", "jul"],
	["agosto", "ago"],
	["septiembre", "sep"],
	["octubre", "oct"],
	["noviembre", "nov"],
	["diciembre", "dic"],
];

const PORTUGUESE_MONTHS: [[&str; 2]; 12] = [
	["janeiro", "jan"],
	["fevereiro", "fev"],
	["março", "mar"],
	["abril", "abr"],
	["maio", "mai"],
	["junho", "jun"],
	["julho", "jul"],
	["agosto", "ago"],
	["setembro", "set"],
	["outubro", "out"],
	["novembro", "nov"],
	["dezembro", "dez"],
];

const FRENCH_MONTHS: [[&str; 2]; 12] = [
	["janvier", "janv"],
	["février", "févr"],
	["mars", "mars"],
	["avril", "avr"],
	["mai", "mai"],
	["juin", "juin"],
	["juillet", "juil"],
	["août", "août"],
	["septembre", "sept"],
	["octobre", "oct"],
	["novembre", "nov"],
	["décembre", "déc"],
];

const INDONESIAN_MONTHS: [[&str; 2]; 12] = [
	["januari", "jan"],
	["februari", "feb"],
	["maret", "mar"],
	["april", "apr"],
	["mei", "mei"],
	["juni", "jun"],
	["juli", "jul"],
	["agustus", "agu"],
	["september", "sep"],
	["oktober", "okt"],
	["november", "nov"],
	["desember", "des"],
];

const TURKISH_MONTHS: [[&str; 2]; 12] = [
	["ocak", "oca"],
	["şubat", "şub"],
	["mart", "mar"],
	["nisan", "nis"],
	["mayıs", "may"],
	["haziran", "haz"],
	["temmuz", "tem"],
	["ağustos", "ağu"],
	["eylül", "eyl"],
	["ekim", "eki"],
	["kasım", "kas"],
	["aralık", "ara"],
];

const ITALIAN_MONTHS: [[&str; 2]; 12] = [
	["gennaio", "gen"],
	["febbraio", "feb"],
	["marzo", "mar"],
	["aprile", "apr"],
	["maggio", "mag"],
	["giugno", "giu"],
	["luglio", "lug"],
	["agosto", "ago"],
	["settembre", "set"],
	["ottobre", "ott"],
	["novembre", "nov"],
	["dicembre", "dic"],
];

// Relative-date vocabulary, as carried by MadaraBase.kt's companion object.
const TODAY_WORDS: [&str; 3] = ["today", "hoje", "hoy"];
const YESTERDAY_WORDS: [&str; 4] = ["yesterday", "ontem", "ayer", "يوم واحد"];
const YEAR_WORDS: [&str; 7] = ["year", "año", "ano", "năm", "yıl", "سنة", "سنوات"];
const MONTH_WORDS: [&str; 7] = ["month", "mes", "tháng", "ay", "شهر", "أشهر", "شهور"];
const WEEK_WORDS: [&str; 6] = ["week", "semana", "tuần", "hafta", "أسبوع", "أسابيع"];
const DAY_WORDS: [&str; 10] = [
	"day", "día", "dia", "jour", "hari", "gün", "ngày", "giorni", "أيام", "天",
];
const HOUR_WORDS: [&str; 10] = [
	"hour",
	"hora",
	"heure",
	"jam",
	"saat",
	"giờ",
	"ore",
	"ساعة",
	"ساعات",
	"小时",
];
const MINUTE_WORDS: [&str; 8] = [
	"minute",
	"minuto",
	"min",
	"menit",
	"dakika",
	"phút",
	"دقيقة",
	"دقائق",
];
const SECOND_WORDS: [&str; 7] =
	["second", "segundo", "sec", "detik", "giây", "ثانية", "ثوان"];

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn java_patterns_translate_to_strftime() {
		assert_eq!(java_pattern_to_strftime("MMMM dd, yyyy"), "%B %d, %Y");
		assert_eq!(java_pattern_to_strftime("d MMM. yyyy"), "%d %b. %Y");
		assert_eq!(java_pattern_to_strftime("yyyy-MM-dd"), "%Y-%m-%d");
		assert_eq!(java_pattern_to_strftime("MM/dd/yyyy"), "%m/%d/%Y");
		assert_eq!(
			java_pattern_to_strftime("dd MMMM yyyy HH:mm"),
			"%d %B %Y %H:%M"
		);
		assert_eq!(java_pattern_to_strftime("MMM d, yyyy"), "%b %d, %Y");
		// Quoted literals survive; 'de' is common in Spanish patterns.
		assert_eq!(
			java_pattern_to_strftime("dd 'de' MMMM 'de' yyyy"),
			"%d de %B de %Y"
		);
	}

	#[test]
	fn absolute_dates_parse_with_the_theme_default() {
		let parser = DateParser::default();
		let parsed = parser.parse("January 12, 2024").expect("date parses");
		assert_eq!(
			parsed.date_naive(),
			NaiveDate::from_ymd_opt(2024, 1, 12).unwrap()
		);
		// Single-digit days are accepted even though the pattern says `dd`.
		assert!(parser.parse("March 3, 2023").is_some());
		assert!(parser.parse("not a date at all").is_none());
	}

	#[test]
	fn locale_month_names_are_translated_before_parsing() {
		let spanish = DateParser::new("dd MMMM yyyy", "es");
		assert_eq!(
			spanish.parse("14 marzo 2024").unwrap().date_naive(),
			NaiveDate::from_ymd_opt(2024, 3, 14).unwrap()
		);
		let turkish = DateParser::new("dd MMMM yyyy", "tr-TR");
		assert_eq!(
			turkish.parse("09 Ağustos 2022").unwrap().date_naive(),
			NaiveDate::from_ymd_opt(2022, 8, 9).unwrap()
		);
		// Without the locale the same string must not silently parse.
		assert!(DateParser::new("dd MMMM yyyy", "en-US")
			.parse("09 Ağustos 2022")
			.is_none());
	}

	#[test]
	fn iso_attribute_values_parse_as_a_fallback() {
		let parser = DateParser::new("MMMM dd, yyyy", "en-US");
		assert_eq!(
			parser.parse("2024-05-06").unwrap().date_naive(),
			NaiveDate::from_ymd_opt(2024, 5, 6).unwrap()
		);
		assert!(parser.parse("2024-05-06T10:11:12+00:00").is_some());
	}

	#[test]
	fn relative_phrases_resolve_against_now() {
		let now = Utc.with_ymd_and_hms(2024, 3, 15, 12, 0, 0).unwrap();
		let day = parse_relative("2 days ago", now).unwrap();
		assert_eq!(day, now - Duration::days(2));
		assert_eq!(
			parse_relative("hace 3 horas", now).unwrap(),
			now - Duration::hours(3)
		);
		assert_eq!(
			parse_relative("5 dakika önce", now).unwrap(),
			now - Duration::minutes(5)
		);
		assert_eq!(
			parse_relative("1 semana", now).unwrap(),
			now - Duration::weeks(1)
		);
		assert_eq!(
			parse_relative("Today", now).unwrap().date_naive(),
			NaiveDate::from_ymd_opt(2024, 3, 15).unwrap()
		);
		assert_eq!(
			parse_relative("ontem", now).unwrap().date_naive(),
			NaiveDate::from_ymd_opt(2024, 3, 14).unwrap()
		);
		// Month arithmetic clamps instead of overflowing.
		let end_of_march = Utc.with_ymd_and_hms(2024, 3, 31, 0, 0, 0).unwrap();
		assert_eq!(
			parse_relative("1 month ago", end_of_march)
				.unwrap()
				.date_naive(),
			NaiveDate::from_ymd_opt(2024, 2, 29).unwrap()
		);
		assert!(parse_relative("no numbers here", now).is_none());
		assert!(parse_relative("7 fortnights", now).is_none());
	}
}
