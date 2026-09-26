use std::fs;
use std::path::PathBuf;

use windows::Win32::Foundation::POINT;
use windows::Win32::Graphics::Gdi::{MONITOR_DEFAULTTONULL, MonitorFromPoint};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Config {
    pub position: Option<(i32, i32)>,
    pub compact: bool,
}

fn path() -> Option<PathBuf> {
    let appdata = std::env::var_os("APPDATA")?;
    Some(
        PathBuf::from(appdata)
            .join("HSReconnect")
            .join("config.ini"),
    )
}

impl Config {
    pub fn load() -> Self {
        path()
            .and_then(|p| fs::read_to_string(p).ok())
            .map(|s| Self::parse(&s))
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let Some(p) = path() else { return };
        if let Some(dir) = p.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let _ = fs::write(p, self.serialize());
    }

    pub fn visible_position(&self) -> Option<(i32, i32)> {
        let (x, y) = self.position?;
        let probe = POINT {
            x: x + 40,
            y: y + 12,
        };
        let monitor = unsafe { MonitorFromPoint(probe, MONITOR_DEFAULTTONULL) };
        (!monitor.is_invalid()).then_some((x, y))
    }

    fn parse(s: &str) -> Self {
        let (mut x, mut y, mut compact) = (None, None, false);
        for line in s.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            match key.trim() {
                "x" => x = value.parse().ok(),
                "y" => y = value.parse().ok(),
                "compact" => compact = value == "true",
                _ => {}
            }
        }
        Self {
            position: x.zip(y),
            compact,
        }
    }

    fn serialize(&self) -> String {
        let mut s = format!("compact={}\n", self.compact);
        if let Some((x, y)) = self.position {
            s += &format!("x={x}\ny={y}\n");
        }
        s
    }
}
