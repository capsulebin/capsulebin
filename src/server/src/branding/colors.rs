//! The operator's theme colours: `[branding.colors.light]` and
//! `[branding.colors.dark]`.
//!
//! Each table sets any of the colour tokens of the built-in theme
//! (`src/server/web/src/styles/tokens.css`), the key `x_y` standing for the
//! token `--cb-x-y`; a token left out keeps its built-in value. A value is
//! `#rgb` or `#rrggbb` and nothing else: enough to compute the contrast
//! exactly, and it can never break out of a CSS declaration.
//!
//! A colour pair below the contrast the built-in theme is held to is a
//! warning, not an error ([`Colors::contrast_warnings`]): `serve` logs it,
//! `check` prints it.

use std::fmt;

use serde::Deserialize;

/// The table of these colours, for error messages.
const TABLE: &str = "branding.colors";

/// A colour as `0xRRGGBB`, shown as lowercase `#rrggbb`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color(u32);

impl Color {
    /// `#rgb` or `#rrggbb` with hex digits in either case; nothing around it.
    pub fn parse(value: &str) -> Option<Self> {
        let digits = value.strip_prefix('#')?;
        if !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return None;
        }
        let number = u32::from_str_radix(digits, 16).ok()?;
        match digits.len() {
            6 => Some(Self(number)),
            // Each digit doubled: #abc is #aabbcc.
            3 => Some(Self(
                ((number >> 8) & 0xf) * 0x11_0000 + ((number >> 4) & 0xf) * 0x1100 + (number & 0xf) * 0x11,
            )),
            _ => None,
        }
    }

    /// Relative luminance as WCAG 2.2 defines it.
    fn luminance(self) -> f64 {
        let channel = |shift: u32| {
            let value = f64::from((self.0 >> shift) & 0xff) / 255.0;
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(16) + 0.7152 * channel(8) + 0.0722 * channel(0)
    }

    /// Contrast ratio of two colours as WCAG 2.2 defines it, 1 to 21.
    fn contrast(self, other: Self) -> f64 {
        let (a, b) = (self.luminance(), other.luminance());
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }
}

impl fmt::Display for Color {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "#{:06x}", self.0)
    }
}

/// One of the two themes of the interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    Light,
    Dark,
}

impl Theme {
    pub const ALL: [Self; 2] = [Self::Light, Self::Dark];

    /// The name of the theme, also the key of its table.
    pub fn name(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }
}

/// A colour token of the built-in theme with its built-in values.
struct Token {
    /// The key in `[branding.colors.<theme>]`: `x_y` for `--cb-x-y`.
    key: &'static str,
    light: Color,
    dark: Color,
}

impl Token {
    fn built_in(&self, theme: Theme) -> Color {
        match theme {
            Theme::Light => self.light,
            Theme::Dark => self.dark,
        }
    }
}

const fn token(key: &'static str, light: u32, dark: u32) -> Token {
    Token {
        key,
        light: Color(light),
        dark: Color(dark),
    }
}

/// Every configurable token, in the order of `tokens.css`, with its built-in
/// values. Kept equal to all four palette blocks of `tokens.css` by a test;
/// `--cb-shadow-card` is not a plain colour and stays built-in.
const TOKENS: [Token; 18] = [
    token("bg", 0xf2f6f5, 0x0c1413),
    token("surface", 0xffffff, 0x131e1d),
    token("surface_sunken", 0xf2f5f5, 0x0f1a19),
    token("border", 0xdbe3e2, 0x263735),
    token("border_strong", 0x7d8e8c, 0x587471),
    token("text", 0x0d1b1a, 0xe6efee),
    token("text_muted", 0x4a5c5a, 0x9db0ae),
    token("accent", 0x0f766e, 0x2dd4bf),
    token("accent_hover", 0x115e59, 0x5eead4),
    token("accent_text", 0xffffff, 0x08201e),
    token("danger", 0xb42318, 0xf97066),
    token("warning", 0xb45309, 0xf79009),
    token("success", 0x067647, 0x47cd89),
    token("focus_ring", 0x0f766e, 0x2dd4bf),
    token("tint_accent", 0xe6f2f1, 0x10322e),
    token("tint_danger", 0xfdecea, 0x33150f),
    token("tint_warning", 0xfdf3e5, 0x2e1e05),
    token("tint_success", 0xe6f4ee, 0x0e2a1d),
];

/// Foregrounds on backgrounds that must hold `min_ratio`.
struct ContrastPair {
    foregrounds: &'static [&'static str],
    backgrounds: &'static [&'static str],
    min_ratio: f64,
}

const GROUNDS: &[&str] = &["bg", "surface", "surface_sunken"];
const TINTS: &[&str] = &["tint_accent", "tint_danger", "tint_warning", "tint_success"];

/// The pairs and thresholds the built-in theme is held to (WCAG 2.2: 4.5:1
/// for text, 3:1 for control outlines and the focus ring). Kept equal to
/// `src/server/web/tests/fixtures/contrast-pairs.json`, which
/// `contrast.test.ts` checks `tokens.css` against, by a test.
const CONTRAST_PAIRS: &[ContrastPair] = &[
    ContrastPair {
        foregrounds: &["text", "text_muted", "accent", "danger", "warning", "success"],
        backgrounds: GROUNDS,
        min_ratio: 4.5,
    },
    ContrastPair {
        foregrounds: &["accent_text"],
        backgrounds: &["accent"],
        min_ratio: 4.5,
    },
    ContrastPair {
        foregrounds: &["accent"],
        backgrounds: &["tint_accent"],
        min_ratio: 4.5,
    },
    ContrastPair {
        foregrounds: &["danger"],
        backgrounds: &["tint_danger"],
        min_ratio: 4.5,
    },
    ContrastPair {
        foregrounds: &["warning"],
        backgrounds: &["tint_warning"],
        min_ratio: 4.5,
    },
    ContrastPair {
        foregrounds: &["success"],
        backgrounds: &["tint_success"],
        min_ratio: 4.5,
    },
    ContrastPair {
        foregrounds: &["text"],
        backgrounds: TINTS,
        min_ratio: 4.5,
    },
    ContrastPair {
        foregrounds: &["border_strong"],
        backgrounds: GROUNDS,
        min_ratio: 3.0,
    },
    ContrastPair {
        foregrounds: &["focus_ring"],
        backgrounds: &["bg"],
        min_ratio: 3.0,
    },
];

/// `[branding.colors]`: the configured tokens of each theme, checked and
/// normalised while the configuration is parsed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Colors {
    light: Vec<(&'static str, Color)>,
    dark: Vec<(&'static str, Color)>,
}

impl Colors {
    /// The configured tokens of `theme` as (key, colour), in the order of
    /// `tokens.css`; a token without a configured value is not included.
    pub fn configured(&self, theme: Theme) -> &[(&'static str, Color)] {
        match theme {
            Theme::Light => &self.light,
            Theme::Dark => &self.dark,
        }
    }

    /// No colour configured in either theme.
    pub fn is_empty(&self) -> bool {
        self.light.is_empty() && self.dark.is_empty()
    }

    /// The stylesheet that sets the configured tokens over the built-in theme,
    /// or `None` without any. It is linked after the bundled stylesheet, so
    /// each block wins over its built-in counterpart: the light tokens for a
    /// light system preference, the dark ones for a dark one (the two media
    /// queries are exact complements, so a token set for one theme never
    /// reaches the other), then the manual choice of either theme, whose
    /// selectors are more specific. A theme without configured tokens has no
    /// blocks. The values are `#rrggbb`, so nothing can leave a declaration.
    pub fn stylesheet(&self) -> Option<String> {
        if self.is_empty() {
            return None;
        }
        let block = |selector: &str, theme: Theme, indent: &str| {
            let declarations: String = self
                .configured(theme)
                .iter()
                .map(|(key, color)| format!("{indent}    --cb-{}: {color};\n", key.replace('_', "-")))
                .collect();
            format!("{indent}{selector} {{\n{declarations}{indent}}}\n")
        };
        let mut css = String::new();
        for (media, theme) in [
            ("@media not all and (prefers-color-scheme: dark)", Theme::Light),
            ("@media (prefers-color-scheme: dark)", Theme::Dark),
        ] {
            if !self.configured(theme).is_empty() {
                css.push_str(&format!("{media} {{\n{}}}\n", block(":root", theme, "    ")));
            }
        }
        for theme in Theme::ALL {
            if !self.configured(theme).is_empty() {
                css.push_str(&block(&format!(":root[data-theme=\"{}\"]", theme.name()), theme, ""));
            }
        }
        Some(css)
    }

    /// The colour of `key` in `theme`: the configured one, else the built-in.
    fn color(&self, theme: Theme, key: &str) -> Option<Color> {
        self.configured(theme)
            .iter()
            .find(|(configured, _)| *configured == key)
            .map(|&(_, color)| color)
            .or_else(|| TOKENS.iter().find(|token| token.key == key).map(|token| token.built_in(theme)))
    }

    /// Every pair of [`CONTRAST_PAIRS`] below its threshold, per theme on the
    /// merged palette. The built-in palette has none.
    pub fn contrast_warnings(&self) -> Vec<ContrastWarning> {
        let mut warnings = Vec::new();
        for theme in Theme::ALL {
            for pair in CONTRAST_PAIRS {
                for &foreground in pair.foregrounds {
                    for &background in pair.backgrounds {
                        let (Some(fore), Some(back)) = (self.color(theme, foreground), self.color(theme, background)) else {
                            continue;
                        };
                        let ratio = fore.contrast(back);
                        if ratio < pair.min_ratio {
                            warnings.push(ContrastWarning {
                                theme,
                                foreground,
                                background,
                                ratio,
                                min_ratio: pair.min_ratio,
                            });
                        }
                    }
                }
            }
        }
        warnings
    }

    /// Reads `[branding.colors]`; every error names the full key.
    fn from_value(value: toml::Value) -> Result<Self, String> {
        let toml::Value::Table(table) = value else {
            return Err(format!("{TABLE} must be a table with the tables light and dark"));
        };
        let mut colors = Self::default();
        for (name, value) in table {
            let theme = Theme::ALL
                .into_iter()
                .find(|theme| theme.name() == name)
                .ok_or_else(|| format!("{TABLE}.{name} is unknown; [{TABLE}] takes the tables light and dark"))?;
            let configured = theme_colors(theme, value)?;
            match theme {
                Theme::Light => colors.light = configured,
                Theme::Dark => colors.dark = configured,
            }
        }
        Ok(colors)
    }
}

/// Reads `[branding.colors.<theme>]` into (key, colour) in the order of
/// [`TOKENS`].
fn theme_colors(theme: Theme, value: toml::Value) -> Result<Vec<(&'static str, Color)>, String> {
    let table_key = format!("{TABLE}.{}", theme.name());
    let toml::Value::Table(table) = value else {
        return Err(format!("{table_key} must be a table"));
    };
    if let Some(unknown) = table.keys().find(|key| !TOKENS.iter().any(|token| token.key == key.as_str())) {
        let keys: Vec<&str> = TOKENS.iter().map(|token| token.key).collect();
        return Err(format!("{table_key}.{unknown} is unknown; [{table_key}] takes {}", keys.join(", ")));
    }
    TOKENS
        .iter()
        .filter_map(|token| table.get(token.key).map(|value| (token.key, value)))
        .map(|(key, value)| {
            value
                .as_str()
                .and_then(Color::parse)
                .map(|color| (key, color))
                .ok_or_else(|| format!("{table_key}.{key} must be a colour as #rgb or #rrggbb"))
        })
        .collect()
}

impl<'de> Deserialize<'de> for Colors {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::from_value(toml::Value::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

/// A colour pair of the merged palette below its threshold.
#[derive(Debug, Clone, PartialEq)]
pub struct ContrastWarning {
    pub theme: Theme,
    pub foreground: &'static str,
    pub background: &'static str,
    pub ratio: f64,
    pub min_ratio: f64,
}

impl fmt::Display for ContrastWarning {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let theme = self.theme.name();
        write!(
            formatter,
            "{theme} theme: {TABLE}.{theme}.{} on {TABLE}.{theme}.{} has a contrast of {:.2}:1, below {}:1",
            self.foreground, self.background, self.ratio, self.min_ratio
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(toml: &str) -> Result<Colors, String> {
        Colors::from_value(toml::from_str::<toml::Value>(toml).expect("test TOML parses"))
    }

    #[test]
    fn colors_are_normalised_to_lowercase_rrggbb() {
        for (value, expected) in [
            ("#ABC", "#aabbcc"),
            ("#abc", "#aabbcc"),
            ("#0f766E", "#0f766e"),
            ("#000", "#000000"),
            ("#FFFFFF", "#ffffff"),
        ] {
            assert_eq!(
                Color::parse(value).map(|color| color.to_string()).as_deref(),
                Some(expected),
                "{value}"
            );
        }
    }

    #[test]
    fn only_rgb_and_rrggbb_are_colours() {
        for value in [
            "",
            "#",
            "red",
            "#12",
            "#1234",
            "#12345",
            "#1234567",
            "#12345678",
            "#ggg",
            "rgb(0,0,0)",
            " #fff",
            "#fff ",
            "#fff;}",
            "fff",
            "#+ff",
            "#-ff",
            "##fff",
            "#ｆｆｆ",
        ] {
            assert_eq!(Color::parse(value), None, "{value:?}");
        }
    }

    /// WCAG 2.2: `#b45309` on `#f2f5f5` is 4.58:1; black on white 21:1.
    #[test]
    fn contrast_follows_wcag() {
        let ratio = Color(0xb45309).contrast(Color(0xf2f5f5));
        assert_eq!(format!("{ratio:.2}"), "4.58");
        assert_eq!(format!("{:.2}", Color(0).contrast(Color(0xffffff))), "21.00");
        assert_eq!(Color(0x123456).contrast(Color(0x123456)), 1.0);
    }

    #[test]
    fn the_built_in_palette_has_no_contrast_warning() {
        assert_eq!(Colors::default().contrast_warnings(), Vec::new());
    }

    /// A tint as dark as its status colour leaves the status text unreadable
    /// on it.
    #[test]
    fn a_tint_equal_to_its_status_colour_is_warned_about() {
        let colors = parse("[light]\ntint_warning = \"#b45309\"\n").expect("valid colours");
        let warnings: Vec<String> = colors.contrast_warnings().iter().map(ToString::to_string).collect();
        assert!(
            warnings.contains(
                &"light theme: branding.colors.light.warning on branding.colors.light.tint_warning has a contrast of 1.00:1, below 4.5:1"
                    .to_owned()
            ),
            "{warnings:?}"
        );
        assert!(colors.contrast_warnings().iter().all(|warning| warning.theme == Theme::Light));
    }

    /// A configured colour is checked against the built-in values of the
    /// tokens left out, in its own theme only.
    #[test]
    fn contrast_is_checked_on_the_merged_palette_per_theme() {
        let colors = parse("[light]\ntext = \"#fff\"\n[dark]\naccent_text = \"#3ad\"\n").expect("valid colours");
        let warnings: Vec<String> = colors.contrast_warnings().iter().map(ToString::to_string).collect();
        assert_eq!(
            warnings,
            [
                "light theme: branding.colors.light.text on branding.colors.light.bg has a contrast of 1.09:1, below 4.5:1",
                "light theme: branding.colors.light.text on branding.colors.light.surface has a contrast of 1.00:1, below 4.5:1",
                "light theme: branding.colors.light.text on branding.colors.light.surface_sunken has a contrast of 1.10:1, below 4.5:1",
                "light theme: branding.colors.light.text on branding.colors.light.tint_accent has a contrast of 1.15:1, below 4.5:1",
                "light theme: branding.colors.light.text on branding.colors.light.tint_danger has a contrast of 1.14:1, below 4.5:1",
                "light theme: branding.colors.light.text on branding.colors.light.tint_warning has a contrast of 1.10:1, below 4.5:1",
                "light theme: branding.colors.light.text on branding.colors.light.tint_success has a contrast of 1.13:1, below 4.5:1",
                "dark theme: branding.colors.dark.accent_text on branding.colors.dark.accent has a contrast of 1.42:1, below 4.5:1",
            ]
        );

        let colors = parse("[light]\nfocus_ring = \"#eee\"\nborder_strong = \"#ccc\"\n").expect("valid colours");
        let warnings: Vec<(&str, &str, f64)> = colors
            .contrast_warnings()
            .iter()
            .map(|warning| (warning.foreground, warning.background, warning.min_ratio))
            .collect();
        assert_eq!(
            warnings,
            [
                ("border_strong", "bg", 3.0),
                ("border_strong", "surface", 3.0),
                ("border_strong", "surface_sunken", 3.0),
                ("focus_ring", "bg", 3.0),
            ]
        );
        assert!(colors.contrast_warnings().iter().all(|warning| warning.theme == Theme::Light));
    }

    #[test]
    fn configured_tokens_keep_the_order_of_tokens_css() {
        let colors = parse("[dark]\ntint_success = \"#010203\"\nbg = \"#ABC\"\n").expect("valid colours");
        assert_eq!(colors.configured(Theme::Light), []);
        assert_eq!(
            colors.configured(Theme::Dark),
            [("bg", Color(0xaabbcc)), ("tint_success", Color(0x010203))]
        );
        assert!(!colors.is_empty());
        assert!(Colors::default().is_empty());
    }

    #[test]
    fn errors_name_the_full_key() {
        for (toml, expected) in [
            ("[dim]\nbg = \"#fff\"\n", "branding.colors.dim is unknown"),
            ("shadow_card = \"#fff\"\n", "branding.colors.shadow_card is unknown"),
            ("light = \"#fff\"\n", "branding.colors.light must be a table"),
            ("[light]\nshadow_card = \"#fff\"\n", "branding.colors.light.shadow_card is unknown"),
            ("[dark]\nBG = \"#fff\"\n", "branding.colors.dark.BG is unknown"),
            (
                "[dark]\nsurface-sunken = \"#fff\"\n",
                "branding.colors.dark.surface-sunken is unknown",
            ),
            ("[light]\nbg = 1\n", "branding.colors.light.bg must be a colour as #rgb or #rrggbb"),
            (
                "[light.bg]\nx = 1\n",
                "branding.colors.light.bg must be a colour as #rgb or #rrggbb",
            ),
        ] {
            let error = parse(toml).expect_err("refused");
            assert!(error.contains(expected), "{toml:?}: {error}");
        }
        let error = Colors::from_value(toml::Value::String("#fff".to_owned())).expect_err("refused");
        assert!(error.contains("branding.colors must be a table"), "{error}");
    }

    /// Both themes: the light tokens for a light system preference, the dark
    /// ones for a dark one, then both manual choices; keys become tokens.
    #[test]
    fn the_stylesheet_sets_the_configured_tokens_of_both_themes() {
        let colors =
            parse("[light]\naccent = \"#0F766E\"\ndanger = \"#c00\"\n[dark]\naccent_hover = \"#5eead4\"\n").expect("valid colours");
        assert_eq!(
            colors.stylesheet().as_deref(),
            Some(
                "@media not all and (prefers-color-scheme: dark) {\n\
                 \x20   :root {\n\
                 \x20       --cb-accent: #0f766e;\n\
                 \x20       --cb-danger: #cc0000;\n\
                 \x20   }\n\
                 }\n\
                 @media (prefers-color-scheme: dark) {\n\
                 \x20   :root {\n\
                 \x20       --cb-accent-hover: #5eead4;\n\
                 \x20   }\n\
                 }\n\
                 :root[data-theme=\"light\"] {\n\
                 \x20   --cb-accent: #0f766e;\n\
                 \x20   --cb-danger: #cc0000;\n\
                 }\n\
                 :root[data-theme=\"dark\"] {\n\
                 \x20   --cb-accent-hover: #5eead4;\n\
                 }\n"
            )
        );
    }

    /// A theme without configured tokens has no blocks.
    #[test]
    fn the_stylesheet_of_the_light_theme_only() {
        let colors = parse("[light]\nsurface_sunken = \"#eef\"\n[dark]\n").expect("valid colours");
        assert_eq!(
            colors.stylesheet().as_deref(),
            Some(
                "@media not all and (prefers-color-scheme: dark) {\n\
                 \x20   :root {\n\
                 \x20       --cb-surface-sunken: #eeeeff;\n\
                 \x20   }\n\
                 }\n\
                 :root[data-theme=\"light\"] {\n\
                 \x20   --cb-surface-sunken: #eeeeff;\n\
                 }\n"
            )
        );
    }

    #[test]
    fn without_colours_there_is_no_stylesheet() {
        assert_eq!(Colors::default().stylesheet(), None);
        assert_eq!(parse("[light]\n[dark]\n").expect("valid").stylesheet(), None);
    }

    /// The first block of `tokens.css` that starts at `selector`: its colour
    /// tokens as (key, colour). The file has no nested block inside a palette.
    fn css_palette(css: &str, selector: &str) -> Vec<(String, Color)> {
        let start = css.find(selector).unwrap_or_else(|| panic!("{selector} not in tokens.css"));
        let open = start + css[start..].find('{').expect("block opens");
        let close = open + css[open..].find('}').expect("block closes");
        css[open + 1..close]
            .lines()
            .filter_map(|line| {
                let (name, value) = line.trim().strip_prefix("--cb-")?.split_once(':')?;
                let color = Color::parse(value.trim().strip_suffix(';')?.trim())?;
                Some((name.replace('-', "_"), color))
            })
            .collect()
    }

    /// The Rust copy of the built-in palette equals every palette block of
    /// `tokens.css`, token for token and in the same order.
    #[test]
    fn the_built_in_palette_equals_tokens_css() {
        let css = include_str!("../../web/src/styles/tokens.css");
        for (selector, theme) in [
            (":root {", Theme::Light),
            ("@media (prefers-color-scheme: dark)", Theme::Dark),
            (":root[data-theme=\"light\"]", Theme::Light),
            (":root[data-theme=\"dark\"]", Theme::Dark),
        ] {
            let expected: Vec<(String, Color)> = TOKENS.iter().map(|token| (token.key.to_owned(), token.built_in(theme))).collect();
            assert_eq!(css_palette(css, selector), expected, "{selector}");
        }
    }

    /// The commented `[branding.colors.*]` sections of the example
    /// configuration list every key with its built-in value and parse.
    #[test]
    fn the_example_configuration_lists_the_built_in_colours() {
        let example = include_str!("../../../../capsulebin.example.toml");
        let start = example.find("# [branding.colors.light]").expect("colour sections in the example");
        let sections: String = example[start..]
            .lines()
            .map_while(|line| line.strip_prefix('#'))
            .map(|line| format!("{}\n", line.trim()))
            .collect();
        let table: toml::Table = toml::from_str(&sections).expect("uncommented sections parse");
        let colors = Colors::from_value(table["branding"]["colors"].clone()).expect("valid colours");
        for theme in Theme::ALL {
            let expected: Vec<(&str, Color)> = TOKENS.iter().map(|token| (token.key, token.built_in(theme))).collect();
            assert_eq!(colors.configured(theme), expected, "{}", theme.name());
        }
    }

    /// The pairs and thresholds equal the fixture `contrast.test.ts` reads.
    #[test]
    fn the_contrast_pairs_equal_the_shared_fixture() {
        #[derive(serde::Deserialize)]
        struct Fixture {
            pairs: Vec<Pair>,
        }
        #[derive(serde::Deserialize, Debug, PartialEq)]
        struct Pair {
            foreground: Vec<String>,
            background: Vec<String>,
            min_ratio: f64,
        }

        let fixture: Fixture = serde_json::from_str(include_str!("../../web/tests/fixtures/contrast-pairs.json")).expect("fixture parses");
        let ours: Vec<Pair> = CONTRAST_PAIRS
            .iter()
            .map(|pair| Pair {
                foreground: pair.foregrounds.iter().map(|&key| key.to_owned()).collect(),
                background: pair.backgrounds.iter().map(|&key| key.to_owned()).collect(),
                min_ratio: pair.min_ratio,
            })
            .collect();
        assert_eq!(ours, fixture.pairs);
        for pair in CONTRAST_PAIRS {
            for key in pair.foregrounds.iter().chain(pair.backgrounds) {
                assert!(TOKENS.iter().any(|token| token.key == *key), "{key} is not a token");
            }
        }
    }
}
