use ratatui::style::Color;

#[derive(Clone, Copy)]
pub struct Theme {
    pub name: &'static str,
    pub bg: Color,
    pub fg: Color,
    pub accent: Color,
    pub selection: Color,
    pub muted: Color,
    pub green: Color,
    pub yellow: Color,
    pub red: Color,
}

impl Theme {
    pub const DEFAULT: Theme = Theme {
        name: "default",
        bg: Color::Reset,
        fg: Color::Gray,
        accent: Color::Cyan,
        selection: Color::DarkGray,
        muted: Color::DarkGray,
        green: Color::Green,
        yellow: Color::Yellow,
        red: Color::Red,
    };
}

fn hex_color(s: &str) -> Option<Color> {
    let hex = s.trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(Color::Rgb(r, g, b))
}

/// Loads the active Omarchy theme from
/// ~/.local/state/omarchy/current/theme/colors.toml (keys like
/// background/foreground/accent/selection/muted/green/yellow/red).
pub fn system() -> Option<Theme> {
    let home = std::env::var("HOME").ok()?;
    let path = format!("{home}/.local/state/omarchy/current/theme/colors.toml");
    let raw = std::fs::read_to_string(path).ok()?;

    Some(from_raw(&raw))
}

fn from_raw(raw: &str) -> Theme {
    let mut t = Theme::DEFAULT;
    t.name = "omarchy";
    for line in raw.lines() {
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let v = v.trim().trim_matches('"');
        if !v.starts_with('#') || v.len() < 7 {
            continue;
        }
        let Some(c) = hex_color(v) else { continue };
        match k.trim() {
            "background" => t.bg = c,
            "foreground" => t.fg = c,
            "accent" => t.accent = c,
            "selection" => t.selection = c,
            "muted" => t.muted = c,
            "green" => t.green = c,
            "yellow" => t.yellow = c,
            "red" => t.red = c,
            _ => {}
        }
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_ignores_comments_and_keeps_assignments() {
        let theme = from_raw("# comment\n[colors]\naccent = \"#123456\"\n");
        assert_eq!(theme.accent, Color::Rgb(0x12, 0x34, 0x56));
    }
}
