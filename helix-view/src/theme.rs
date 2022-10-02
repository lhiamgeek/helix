use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use anyhow::Context;
use helix_core::hashmap;
use log::warn;
use once_cell::sync::Lazy;
use serde::{Deserialize, Deserializer};
use toml::Value;

use crate::graphics::UnderlineStyle;
pub use crate::graphics::{Color, Modifier, Style};

pub static DEFAULT_THEME: Lazy<Theme> = Lazy::new(|| {
    toml::from_slice(include_bytes!("../../theme.toml")).expect("Failed to parse default theme")
});
pub static BASE16_DEFAULT_THEME: Lazy<Theme> = Lazy::new(|| {
    toml::from_slice(include_bytes!("../../base16_theme.toml"))
        .expect("Failed to parse base 16 default theme")
});

#[derive(Clone, Debug)]
pub struct Loader {
    user_dir: PathBuf,
    default_dir: PathBuf,
}
impl Loader {
    /// Creates a new loader that can load themes from two directories.
    pub fn new<P: AsRef<Path>>(user_dir: P, default_dir: P) -> Self {
        Self {
            user_dir: user_dir.as_ref().join("themes"),
            default_dir: default_dir.as_ref().join("themes"),
        }
    }

    /// Loads a theme first looking in the `user_dir` then in `default_dir`
    pub fn load(&self, name: &str) -> Result<Theme, anyhow::Error> {
        if name == "default" {
            return Ok(self.default());
        }
        if name == "base16_default" {
            return Ok(self.base16_default());
        }
        let filename = format!("{}.toml", name);

        let user_path = self.user_dir.join(&filename);
        let path = if user_path.exists() {
            user_path
        } else {
            self.default_dir.join(filename)
        };

        let data = std::fs::read(&path)?;
        toml::from_slice(data.as_slice()).context("Failed to deserialize theme")
    }

    pub fn read_names(path: &Path) -> Vec<String> {
        std::fs::read_dir(path)
            .map(|entries| {
                entries
                    .filter_map(|entry| {
                        let entry = entry.ok()?;
                        let path = entry.path();
                        (path.extension()? == "toml")
                            .then(|| path.file_stem().unwrap().to_string_lossy().into_owned())
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Lists all theme names available in default and user directory
    pub fn names(&self) -> Vec<String> {
        let mut names = Self::read_names(&self.user_dir);
        names.extend(Self::read_names(&self.default_dir));
        names
    }

    pub fn default_theme(&self, true_color: bool) -> Theme {
        if true_color {
            self.default()
        } else {
            self.base16_default()
        }
    }

    /// Returns the default theme
    pub fn default(&self) -> Theme {
        DEFAULT_THEME.clone()
    }

    /// Returns the alternative 16-color default theme
    pub fn base16_default(&self) -> Theme {
        BASE16_DEFAULT_THEME.clone()
    }
}

#[derive(Clone, Debug)]
pub struct Theme {
    // UI styles are stored in a HashMap
    styles: HashMap<String, Style>,
    // tree-sitter highlight styles are stored in a Vec to optimize lookups
    scopes: Vec<String>,
    highlights: Vec<Style>,
    rainbow_length: usize,
}

impl<'de> Deserialize<'de> for Theme {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let mut styles = HashMap::new();
        let mut scopes = Vec::new();
        let mut highlights = Vec::new();
        let mut rainbow_length = 0;

        if let Ok(mut colors) = HashMap::<String, Value>::deserialize(deserializer) {
            // TODO: alert user of parsing failures in editor
            let palette = colors
                .remove("palette")
                .map(|value| {
                    ThemePalette::try_from(value).unwrap_or_else(|err| {
                        warn!("{}", err);
                        ThemePalette::default()
                    })
                })
                .unwrap_or_default();

            styles.reserve(colors.len());
            scopes.reserve(colors.len());
            highlights.reserve(colors.len());

            for (i, style) in colors
                .remove("rainbow")
                .and_then(|value| match palette.parse_style_array(value) {
                    Ok(styles) => Some(styles),
                    Err(err) => {
                        warn!("{}", err);
                        None
                    }
                })
                .unwrap_or_else(Self::default_rainbow)
                .iter()
                .enumerate()
            {
                let name = format!("rainbow.{}", i);
                styles.insert(name.clone(), *style);
                scopes.push(name);
                highlights.push(*style);
                rainbow_length += 1;
            }

            for (name, style_value) in colors {
                let mut style = Style::default();
                if let Err(err) = palette.parse_style(&mut style, style_value) {
                    warn!("{}", err);
                }

                // these are used both as UI and as highlights
                styles.insert(name.clone(), style);
                scopes.push(name);
                highlights.push(style);
            }
        }

        Ok(Self {
            scopes,
            styles,
            highlights,
            rainbow_length,
        })
    }
}

impl Theme {
    #[inline]
    pub fn highlight(&self, index: usize) -> Style {
        self.highlights[index]
    }

    pub fn get(&self, scope: &str) -> Style {
        self.try_get(scope).unwrap_or_default()
    }

    /// Get the style of a scope, falling back to dot separated broader
    /// scopes. For example if `ui.text.focus` is not defined in the theme,
    /// `ui.text` is tried and then `ui` is tried.
    pub fn try_get(&self, scope: &str) -> Option<Style> {
        std::iter::successors(Some(scope), |s| Some(s.rsplit_once('.')?.0))
            .find_map(|s| self.styles.get(s).copied())
    }

    #[inline]
    pub fn scopes(&self) -> &[String] {
        &self.scopes
    }

    pub fn find_scope_index(&self, scope: &str) -> Option<usize> {
        self.scopes().iter().position(|s| s == scope)
    }

    pub fn is_16_color(&self) -> bool {
        self.styles.iter().all(|(_, style)| {
            [style.fg, style.bg]
                .into_iter()
                .all(|color| !matches!(color, Some(Color::Rgb(..))))
        })
    }

    pub fn rainbow_length(&self) -> usize {
        self.rainbow_length
    }

    pub fn default_rainbow() -> Vec<Style> {
        vec![
            Style::default().fg(Color::Red),
            Style::default().fg(Color::Yellow),
            Style::default().fg(Color::Green),
            Style::default().fg(Color::Blue),
            Style::default().fg(Color::Cyan),
            Style::default().fg(Color::Magenta),
        ]
    }
}

struct ThemePalette {
    palette: HashMap<String, Color>,
}

impl Default for ThemePalette {
    fn default() -> Self {
        Self {
            palette: hashmap! {
                "black".to_string() => Color::Black,
                "red".to_string() => Color::Red,
                "green".to_string() => Color::Green,
                "yellow".to_string() => Color::Yellow,
                "blue".to_string() => Color::Blue,
                "magenta".to_string() => Color::Magenta,
                "cyan".to_string() => Color::Cyan,
                "gray".to_string() => Color::Gray,
                "light-red".to_string() => Color::LightRed,
                "light-green".to_string() => Color::LightGreen,
                "light-yellow".to_string() => Color::LightYellow,
                "light-blue".to_string() => Color::LightBlue,
                "light-magenta".to_string() => Color::LightMagenta,
                "light-cyan".to_string() => Color::LightCyan,
                "light-gray".to_string() => Color::LightGray,
                "white".to_string() => Color::White,
            },
        }
    }
}

impl ThemePalette {
    pub fn new(palette: HashMap<String, Color>) -> Self {
        let ThemePalette {
            palette: mut default,
        } = ThemePalette::default();

        default.extend(palette);
        Self { palette: default }
    }

    pub fn hex_string_to_rgb(s: &str) -> Result<Color, String> {
        if s.starts_with('#') && s.len() >= 7 {
            if let (Ok(red), Ok(green), Ok(blue)) = (
                u8::from_str_radix(&s[1..3], 16),
                u8::from_str_radix(&s[3..5], 16),
                u8::from_str_radix(&s[5..7], 16),
            ) {
                return Ok(Color::Rgb(red, green, blue));
            }
        }

        Err(format!("Theme: malformed hexcode: {}", s))
    }

    fn parse_value_as_str(value: &Value) -> Result<&str, String> {
        value
            .as_str()
            .ok_or(format!("Theme: unrecognized value: {}", value))
    }

    pub fn parse_color(&self, value: Value) -> Result<Color, String> {
        let value = Self::parse_value_as_str(&value)?;

        self.palette
            .get(value)
            .copied()
            .ok_or("")
            .or_else(|_| Self::hex_string_to_rgb(value))
    }

    pub fn parse_modifier(value: &Value) -> Result<Modifier, String> {
        value
            .as_str()
            .and_then(|s| s.parse().ok())
            .ok_or(format!("Theme: invalid modifier: {}", value))
    }

    pub fn parse_underline_style(value: &Value) -> Result<UnderlineStyle, String> {
        value
            .as_str()
            .and_then(|s| s.parse().ok())
            .ok_or(format!("Theme: invalid underline_style: {}", value))
    }

    pub fn parse_style(&self, style: &mut Style, value: Value) -> Result<(), String> {
        if let Value::Table(entries) = value {
            for (name, value) in entries {
                match name.as_str() {
                    "fg" => *style = style.fg(self.parse_color(value)?),
                    "bg" => *style = style.bg(self.parse_color(value)?),
                    "underline_color" => *style = style.underline_color(self.parse_color(value)?),
                    "underline_style" => {
                        warn!("found style");
                        *style = style.underline_style(Self::parse_underline_style(&value)?)
                    }
                    "modifiers" => {
                        let modifiers = value
                            .as_array()
                            .ok_or("Theme: modifiers should be an array")?;

                        for modifier in modifiers {
                            if modifier
                                .as_str()
                                .map_or(false, |modifier| modifier == "underlined")
                            {
                                *style = style.underline_style(UnderlineStyle::Line);
                            } else {
                                *style = style.add_modifier(Self::parse_modifier(modifier)?);
                            }
                        }
                    }
                    _ => return Err(format!("Theme: invalid style attribute: {}", name)),
                }
            }
        } else {
            *style = style.fg(self.parse_color(value)?);
        }
        Ok(())
    }

    /// Parses a TOML array into a [`Vec`] of [`Style`]. If the value cannot be
    /// parsed as an array or if any style in the array cannot be parsed then an
    /// error is returned.
    pub fn parse_style_array(&self, value: Value) -> Result<Vec<Style>, String> {
        let mut styles = Vec::new();

        for v in value
            .as_array()
            .ok_or_else(|| format!("Theme: could not parse value as an array: '{}'", value))?
        {
            let mut style = Style::default();
            self.parse_style(&mut style, v.clone())?;
            styles.push(style);
        }

        Ok(styles)
    }
}

impl TryFrom<Value> for ThemePalette {
    type Error = String;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        let map = match value {
            Value::Table(entries) => entries,
            _ => return Ok(Self::default()),
        };

        let mut palette = HashMap::with_capacity(map.len());
        for (name, value) in map {
            let value = Self::parse_value_as_str(&value)?;
            let color = Self::hex_string_to_rgb(value)?;
            palette.insert(name, color);
        }

        Ok(Self::new(palette))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_style_string() {
        let fg = Value::String("#ffffff".to_string());

        let mut style = Style::default();
        let palette = ThemePalette::default();
        palette.parse_style(&mut style, fg).unwrap();

        assert_eq!(style, Style::default().fg(Color::Rgb(255, 255, 255)));
    }

    #[test]
    fn test_palette() {
        use helix_core::hashmap;
        let fg = Value::String("my_color".to_string());

        let mut style = Style::default();
        let palette =
            ThemePalette::new(hashmap! { "my_color".to_string() => Color::Rgb(255, 255, 255) });
        palette.parse_style(&mut style, fg).unwrap();

        assert_eq!(style, Style::default().fg(Color::Rgb(255, 255, 255)));
    }

    #[test]
    fn test_parse_style_table() {
        let table = toml::toml! {
            "keyword" = {
                fg = "#ffffff",
                bg = "#000000",
                modifiers = ["bold"],
            }
        };

        let mut style = Style::default();
        let palette = ThemePalette::default();
        if let Value::Table(entries) = table {
            for (_name, value) in entries {
                palette.parse_style(&mut style, value).unwrap();
            }
        }

        assert_eq!(
            style,
            Style::default()
                .fg(Color::Rgb(255, 255, 255))
                .bg(Color::Rgb(0, 0, 0))
                .add_modifier(Modifier::BOLD)
        );
    }

    #[test]
    fn test_parse_valid_style_array() {
        let theme = toml::toml! {
            rainbow = ["#ff0000", "#ffa500", "#fff000", { fg = "#00ff00", modifiers = ["bold"] }]
        };

        let palette = ThemePalette::default();

        let rainbow = theme.as_table().unwrap().get("rainbow").unwrap();
        let parse_result = palette.parse_style_array(rainbow.clone());

        assert_eq!(
            Ok(vec![
                Style::default().fg(Color::Rgb(255, 0, 0)),
                Style::default().fg(Color::Rgb(255, 165, 0)),
                Style::default().fg(Color::Rgb(255, 240, 0)),
                Style::default()
                    .fg(Color::Rgb(0, 255, 0))
                    .add_modifier(Modifier::BOLD),
            ]),
            parse_result
        )
    }

    #[test]
    fn test_parse_invalid_style_array() {
        let palette = ThemePalette::default();

        let theme = toml::toml! { invalid_hex_code = ["#f00"] };
        let invalid_hex_code = theme.as_table().unwrap().get("invalid_hex_code").unwrap();
        let parse_result = palette.parse_style_array(invalid_hex_code.clone());

        assert_eq!(
            Err("Theme: malformed hexcode: #f00".to_string()),
            parse_result
        );

        let theme = toml::toml! { not_an_array = { red = "#ff0000" } };
        let not_an_array = theme.as_table().unwrap().get("not_an_array").unwrap();
        let parse_result = palette.parse_style_array(not_an_array.clone());

        assert_eq!(
            Err("Theme: could not parse value as an array: 'red = \"#ff0000\"\n'".to_string()),
            parse_result
        )
    }
}
