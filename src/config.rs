//! Settings: a `config.toml` file of flat `key = value` entries, read by a
//! small parser for that subset of TOML (strings in double or single quotes,
//! integers, comments).
//!
//! The file lives in `%APPDATA%\simple-screen-grabber`, or next to the
//! executable in portable mode (a file named `portable` beside it). It is
//! only written when the user changes a setting, from a commented template;
//! a change rewrites only its own line. Missing or invalid values fall back
//! to the defaults, and each problem is reported.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::annotate::{COLORS, DEFAULT_COLOR};
use crate::hotkey::Hotkey;

const FILE_NAME: &str = "config.toml";
const PORTABLE_MARKER: &str = "portable";
const APP_FOLDER: &str = "simple-screen-grabber";

/// Stroke width limits in DIPs (at 100% display scale).
const MIN_STROKE: i64 = 1;
const MAX_STROKE: i64 = 20;

/// The settings.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub hotkey: Hotkey,
    /// Where screenshots are saved; `None` for the Windows Screenshots
    /// folder.
    pub save_folder: Option<PathBuf>,
    /// Index of the default annotation color in [`COLORS`].
    pub color: usize,
    /// Default stroke width in DIPs, multiplied by the display scale.
    pub stroke_width: f32,
    /// Whether captures include the mouse pointer.
    pub include_cursor: bool,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            hotkey: Hotkey::DEFAULT,
            save_folder: None,
            color: DEFAULT_COLOR,
            stroke_width: 3.0,
            include_cursor: false,
        }
    }
}

/// Where the settings file is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Location {
    pub folder: PathBuf,
    /// Portable mode: the settings sit next to the executable and nothing
    /// is written elsewhere.
    pub portable: bool,
}

impl Location {
    /// Portable mode when a `portable` file sits next to the executable;
    /// otherwise `%APPDATA%\simple-screen-grabber`.
    pub fn find() -> Location {
        let exe_folder = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(Path::to_path_buf));
        if let Some(folder) = exe_folder.filter(|f| f.join(PORTABLE_MARKER).is_file()) {
            return Location {
                folder,
                portable: true,
            };
        }
        let appdata = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_default();
        Location {
            folder: appdata.join(APP_FOLDER),
            portable: false,
        }
    }

    pub fn file(&self) -> PathBuf {
        self.folder.join(FILE_NAME)
    }

    /// The settings file's modification time and size, to tell a real
    /// change from other activity in its folder.
    pub fn stamp(&self) -> Option<(SystemTime, u64)> {
        let meta = std::fs::metadata(self.file()).ok()?;
        Some((meta.modified().ok()?, meta.len()))
    }

    /// Reads the settings; a missing file gives the defaults.
    pub fn load(&self) -> (Config, Vec<String>) {
        match std::fs::read_to_string(self.file()) {
            Ok(text) => parse(&text, &self.folder),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                (Config::default(), Vec::new())
            }
            Err(err) => (
                Config::default(),
                vec![format!("{FILE_NAME} could not be read: {err}")],
            ),
        }
    }

    /// Writes one setting to the file, creating it from the template if
    /// needed. Only that setting's line changes: comments and the other
    /// lines stay as the user wrote them.
    pub fn save(&self, change: &Change) -> std::io::Result<()> {
        let file = self.ensure_file()?;
        let text = std::fs::read_to_string(&file)?;
        let (key, literal) = change.entry();
        std::fs::write(&file, set_value(&text, key, &literal))
    }

    /// The settings file, written from the template if it does not exist.
    pub fn ensure_file(&self) -> std::io::Result<PathBuf> {
        let file = self.file();
        if !file.exists() {
            std::fs::create_dir_all(&self.folder)?;
            std::fs::write(&file, template())?;
        }
        Ok(file)
    }
}

/// The commented settings file written on first use.
pub fn template() -> String {
    let colors: Vec<String> = COLORS.iter().map(|(_, n)| n.to_lowercase()).collect();
    format!(
        "# Simple Screen Grabber settings. Changes apply when the file is saved.

# Global shortcut that starts a capture, such as \"Ctrl+Alt+S\" or
# \"Shift+PrintScreen\". Modifiers: Ctrl, Alt, Shift, Win.
hotkey = \"{hotkey}\"

# Folder for saved screenshots, in single quotes so backslashes stay as
# they are, for example 'D:\\Screenshots'. Empty: the Windows Screenshots
# folder (Pictures\\Screenshots). A relative path starts at this file's
# folder.
save_folder = ''

# Default annotation color: {colors}.
color = \"{color}\"

# Default stroke width in pixels at 100% display scale, {min} to {max}.
stroke_width = {width}
# Include the mouse pointer in captures: true or false.
include_cursor = false
",
        hotkey = Hotkey::DEFAULT,
        colors = colors.join(", "),
        color = colors[DEFAULT_COLOR],
        min = MIN_STROKE,
        max = MAX_STROKE,
        width = Config::default().stroke_width,
    )
}

/// A change to one setting, made in the settings window.
#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    Hotkey(Hotkey),
    /// `None` for the Windows Screenshots folder.
    SaveFolder(Option<PathBuf>),
    Color(usize),
    StrokeWidth(i64),
}

impl Change {
    /// The key and its value as written in the file.
    fn entry(&self) -> (&'static str, String) {
        match self {
            Change::Hotkey(hotkey) => ("hotkey", format!("\"{hotkey}\"")),
            Change::SaveFolder(None) => ("save_folder", "''".into()),
            Change::SaveFolder(Some(path)) => ("save_folder", quote(&path.to_string_lossy())),
            Change::Color(index) => {
                let name = COLORS[(*index).min(COLORS.len() - 1)].1.to_lowercase();
                ("color", format!("\"{name}\""))
            }
            Change::StrokeWidth(width) => (
                "stroke_width",
                (*width).clamp(MIN_STROKE, MAX_STROKE).to_string(),
            ),
        }
    }
}

/// The stroke width limits, for the settings window.
pub const STROKE_RANGE: std::ops::RangeInclusive<i64> = MIN_STROKE..=MAX_STROKE;

/// Text as a TOML string: single quotes keep backslashes as they are;
/// double quotes with escapes when the text has a single quote.
fn quote(text: &str) -> String {
    if !text.contains('\'') && !text.contains('\n') {
        format!("'{text}'")
    } else {
        let escaped = text
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n");
        format!("\"{escaped}\"")
    }
}

/// `text` with the line of `key` set to `key = literal`, or the entry
/// added at the end when the key is missing. Commented lines and the
/// other settings are kept.
fn set_value(text: &str, key: &str, literal: &str) -> String {
    let entry = format!("{key} = {literal}");
    let mut found = false;
    let mut out: Vec<String> = text
        .lines()
        .map(|line| {
            let is_key = line
                .trim_start()
                .strip_prefix(key)
                .is_some_and(|rest| rest.trim_start().starts_with('='));
            if is_key && !found {
                found = true;
                entry.clone()
            } else {
                line.to_owned()
            }
        })
        .collect();
    if !found {
        if out.last().is_some_and(|l| !l.trim().is_empty()) {
            out.push(String::new());
        }
        out.push(entry);
    }
    let mut result = out.join("\n");
    result.push('\n');
    result
}

/// A value of the TOML subset.
#[derive(Debug, PartialEq)]
enum Value {
    Text(String),
    Integer(i64),
    Bool(bool),
}

/// Parses the settings. Relative folders start at `base`. Returns the
/// settings and a description of each problem found.
pub fn parse(text: &str, base: &Path) -> (Config, Vec<String>) {
    let mut config = Config::default();
    let mut problems = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for (number, raw) in text.lines().enumerate() {
        let line = raw.trim();
        let number = number + 1;
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            problems.push(format!("line {number}: sections are not used"));
            continue;
        }
        let Some((key, rest)) = line.split_once('=') else {
            problems.push(format!("line {number}: expected key = value"));
            continue;
        };
        let key = key.trim();
        let value = match value(rest.trim()) {
            Ok(value) => value,
            Err(err) => {
                problems.push(format!("line {number}: {err}"));
                continue;
            }
        };
        if seen.iter().any(|k| k == key) {
            problems.push(format!("line {number}: {key} is set twice"));
            continue;
        }
        seen.push(key.to_owned());
        let applied = match (key, value) {
            ("hotkey", Value::Text(t)) => Hotkey::parse(&t).map(|h| config.hotkey = h),
            ("save_folder", Value::Text(t)) => {
                let t = t.trim();
                config.save_folder = (!t.is_empty()).then(|| base.join(expand_variables(t)));
                Ok(())
            }
            ("color", Value::Text(t)) => COLORS
                .iter()
                .position(|(_, name)| name.eq_ignore_ascii_case(t.trim()))
                .map(|i| config.color = i)
                .ok_or_else(|| format!("\"{t}\" is not one of the colors")),
            ("stroke_width", Value::Integer(n)) if (MIN_STROKE..=MAX_STROKE).contains(&n) => {
                config.stroke_width = n as f32;
                Ok(())
            }
            ("stroke_width", Value::Integer(n)) => {
                Err(format!("{n} is outside {MIN_STROKE} to {MAX_STROKE}"))
            }
            ("include_cursor", Value::Bool(on)) => {
                config.include_cursor = on;
                Ok(())
            }
            ("include_cursor", _) => Err("expected true or false".into()),
            ("hotkey" | "save_folder" | "color", Value::Integer(_) | Value::Bool(_)) => {
                Err("expected text in quotes".into())
            }
            ("stroke_width", Value::Text(_) | Value::Bool(_)) => Err("expected a number".into()),
            (other, _) => Err(format!("{other} is not a setting")),
        };
        if let Err(err) = applied {
            problems.push(format!("line {number}: {key}: {err}"));
        }
    }
    (config, problems)
}

/// Parses the value after `=`, with an optional trailing comment.
fn value(text: &str) -> Result<Value, String> {
    let mut chars = text.char_indices();
    let rest_after = |end: usize| -> Result<(), String> {
        let rest = text[end..].trim();
        if rest.is_empty() || rest.starts_with('#') {
            Ok(())
        } else {
            Err(format!("unexpected \"{rest}\" after the value"))
        }
    };
    match chars.next() {
        Some((_, '"')) => {
            let mut out = String::new();
            let mut escaped = false;
            for (i, c) in chars {
                if escaped {
                    out.push(match c {
                        'n' => '\n',
                        't' => '\t',
                        '\\' => '\\',
                        '"' => '"',
                        other => {
                            return Err(format!(
                                "unknown escape \\{other}; use single quotes for paths"
                            ))
                        }
                    });
                    escaped = false;
                } else if c == '\\' {
                    escaped = true;
                } else if c == '"' {
                    rest_after(i + 1)?;
                    return Ok(Value::Text(out));
                } else {
                    out.push(c);
                }
            }
            Err("missing closing quote".into())
        }
        Some((_, '\'')) => {
            let end = text[1..]
                .find('\'')
                .ok_or_else(|| "missing closing quote".to_string())?;
            rest_after(end + 2)?;
            Ok(Value::Text(text[1..end + 1].to_owned()))
        }
        Some(_) => {
            let number = text.split('#').next().unwrap_or("").trim();
            match number {
                "true" => return Ok(Value::Bool(true)),
                "false" => return Ok(Value::Bool(false)),
                _ => {}
            }
            number
                .replace('_', "")
                .parse::<i64>()
                .map(Value::Integer)
                .map_err(|_| format!("\"{number}\" is not a value; put text in quotes"))
        }
        None => Err("missing value".into()),
    }
}

/// Expands `%NAME%` environment variables, like Windows paths do.
fn expand_variables(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) => {
                let name = &after[..end];
                match std::env::var(name) {
                    Ok(value) if !name.is_empty() => out.push_str(&value),
                    _ => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = &after[end + 1..];
            }
            None => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_ok(text: &str) -> Config {
        let (config, problems) = parse(text, Path::new(r"C:\base"));
        assert!(problems.is_empty(), "{problems:?}");
        config
    }

    #[test]
    fn the_template_gives_the_defaults() {
        assert_eq!(parse_ok(&template()), Config::default());
    }

    #[test]
    fn reads_every_setting() {
        let config = parse_ok(
            "hotkey = \"Ctrl+Shift+X\"  # comment\n\
             save_folder = 'D:\\Shots'\n\
             color = \"Blue\"\n\
             stroke_width = 5\n",
        );
        assert_eq!(config.hotkey, Hotkey::parse("Ctrl+Shift+X").unwrap());
        assert_eq!(config.save_folder, Some(PathBuf::from(r"D:\Shots")));
        assert_eq!(config.color, 4);
        assert_eq!(config.stroke_width, 5.0);
    }

    #[test]
    fn the_cursor_setting_takes_true_or_false() {
        assert!(parse_ok("include_cursor = true").include_cursor);
        assert!(!parse_ok("include_cursor = false # no").include_cursor);
        let (config, problems) = parse("include_cursor = 1", Path::new("C:\\"));
        assert!(!config.include_cursor);
        assert!(problems[0].contains("true or false"), "{problems:?}");
    }

    #[test]
    fn relative_folders_start_at_the_settings_folder() {
        let config = parse_ok("save_folder = \"shots\"");
        assert_eq!(config.save_folder, Some(PathBuf::from(r"C:\base\shots")));
    }

    #[test]
    fn double_quoted_strings_have_escapes() {
        let config = parse_ok(r#"save_folder = "D:\\My \"Shots\"""#);
        assert_eq!(config.save_folder, Some(PathBuf::from(r#"D:\My "Shots""#)));
    }

    #[test]
    fn environment_variables_expand() {
        std::env::set_var("SSG_TEST_DIR", r"E:\x");
        let config = parse_ok("save_folder = '%SSG_TEST_DIR%\\y'");
        assert_eq!(config.save_folder, Some(PathBuf::from(r"E:\x\y")));
        assert_eq!(
            expand_variables("100% %NOT_SET_SSG% 5%"),
            "100% %NOT_SET_SSG% 5%"
        );
    }

    #[test]
    fn problems_keep_the_defaults_and_say_where() {
        let (config, problems) = parse(
            "hotkey = \"S\"\n\
             color = \"pink\"\n\
             stroke_width = 99\n\
             stroke_width = 2\n\
             zoom = 3\n\
             [section]\n\
             save_folder = \"D:\\x\"\n\
             broken\n",
            Path::new("C:\\"),
        );
        assert_eq!(config, Config::default());
        assert_eq!(problems.len(), 8, "{problems:?}");
        assert!(problems[0].starts_with("line 1: hotkey"));
        assert!(problems[3].contains("set twice"));
    }

    #[test]
    fn changes_rewrite_only_their_line() {
        let text = "# mine\nhotkey = \"Ctrl+Alt+S\"  # old\n# keep\ncolor = \"red\"\n";
        let changed = set_value(text, "color", "\"blue\"");
        assert_eq!(
            changed,
            "# mine\nhotkey = \"Ctrl+Alt+S\"  # old\n# keep\ncolor = \"blue\"\n"
        );
        let added = set_value("# only a comment", "stroke_width", "7");
        assert_eq!(added, "# only a comment\n\nstroke_width = 7\n");
        // A commented-out entry is not the setting.
        let commented = set_value("# color = \"red\"\n", "color", "\"green\"");
        assert!(commented.starts_with("# color = \"red\"\n"));
        assert!(commented.ends_with("color = \"green\"\n"));
    }

    #[test]
    fn every_change_reads_back() {
        let changes = [
            Change::Hotkey(Hotkey::parse("Ctrl+Shift+PrintScreen").unwrap()),
            Change::SaveFolder(Some(PathBuf::from(r"D:\Shots"))),
            Change::Color(5),
            Change::StrokeWidth(9),
        ];
        let mut text = template();
        for change in &changes {
            let (key, literal) = change.entry();
            text = set_value(&text, key, &literal);
        }
        let config = parse_ok(&text);
        assert_eq!(
            config.hotkey,
            Hotkey::parse("Ctrl+Shift+PrintScreen").unwrap()
        );
        assert_eq!(config.save_folder, Some(PathBuf::from(r"D:\Shots")));
        assert_eq!(config.color, 5);
        assert_eq!(config.stroke_width, 9.0);
        let (key, literal) = Change::SaveFolder(None).entry();
        assert_eq!(parse_ok(&set_value(&text, key, &literal)).save_folder, None);
    }

    #[test]
    fn folders_with_quotes_survive() {
        let path = r"D:\Bob's \x";
        let config = parse_ok(&format!("save_folder = {}", quote(path)));
        assert_eq!(config.save_folder, Some(PathBuf::from(path)));
    }

    #[test]
    fn values_need_quotes_or_numbers() {
        assert_eq!(value("12 # x"), Ok(Value::Integer(12)));
        assert_eq!(value("'a#b' # c"), Ok(Value::Text("a#b".into())));
        assert!(value("abc").is_err());
        assert!(value("\"open").is_err());
        assert!(value("'a' b").is_err());
    }
}
