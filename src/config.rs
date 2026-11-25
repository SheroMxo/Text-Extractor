use std::fs;
use std::path::PathBuf;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyNameTextW, MapVirtualKeyW, MAPVK_VK_TO_VSC, MOD_ALT, MOD_CONTROL, MOD_SHIFT, MOD_WIN,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HotkeyConfig {
    pub win: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub vk: u32,
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self {
            win: false,
            ctrl: true,
            alt: false,
            shift: false,
            vk: 0x53, // 'S'
        }
    }
}

impl HotkeyConfig {
    pub fn modifiers_flags(&self) -> u32 {
        let mut m = 0u32;
        if self.win {
            m |= MOD_WIN.0;
        }
        if self.ctrl {
            m |= MOD_CONTROL.0;
        }
        if self.alt {
            m |= MOD_ALT.0;
        }
        if self.shift {
            m |= MOD_SHIFT.0;
        }
        m
    }

    pub fn display(&self) -> String {
        let mut parts = Vec::new();
        if self.win {
            parts.push("Win".to_string());
        }
        if self.ctrl {
            parts.push("Ctrl".to_string());
        }
        if self.alt {
            parts.push("Alt".to_string());
        }
        if self.shift {
            parts.push("Shift".to_string());
        }
        parts.push(vk_display_name(self.vk));
        parts.join(" + ")
    }

    pub fn serialize(&self) -> String {
        let mut mods = Vec::new();
        if self.win {
            mods.push("win");
        }
        if self.ctrl {
            mods.push("ctrl");
        }
        if self.alt {
            mods.push("alt");
        }
        if self.shift {
            mods.push("shift");
        }
        format!("{}:{}", mods.join("+"), self.vk)
    }

    pub fn parse(s: &str) -> Option<Self> {
        let (mods_str, vk_str) = s.split_once(':')?;
        let vk: u32 = vk_str.trim().parse().ok()?;
        let mut cfg = Self {
            win: false,
            ctrl: false,
            alt: false,
            shift: false,
            vk,
        };
        for m in mods_str.split('+') {
            match m.trim().to_lowercase().as_str() {
                "win" => cfg.win = true,
                "ctrl" => cfg.ctrl = true,
                "alt" => cfg.alt = true,
                "shift" => cfg.shift = true,
                _ => {}
            }
        }
        if !cfg.win && !cfg.ctrl && !cfg.alt && !cfg.shift {
            return None;
        }
        Some(cfg)
    }
}

fn vk_display_name(vk: u32) -> String {
    if (0x41..=0x5A).contains(&vk) || (0x30..=0x39).contains(&vk) {
        return ((vk as u8) as char).to_string();
    }
    if (0x70..=0x87).contains(&vk) {
        return format!("F{}", vk - 0x70 + 1);
    }
    unsafe {
        let scan = MapVirtualKeyW(vk, MAPVK_VK_TO_VSC);
        if scan != 0 {
            let mut buf = [0u16; 64];
            let len = GetKeyNameTextW((scan << 16) as i32, &mut buf);
            if len > 0 {
                return String::from_utf16_lossy(&buf[..len as usize]);
            }
        }
    }
    format!("0x{vk:02X}")
}

#[derive(Clone, Debug, Default)]
pub struct AppConfig {
    pub lang: Option<String>,
    pub hotkey: HotkeyConfig,
}

fn config_path() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("config.txt")))
        .unwrap_or_else(|| PathBuf::from("config.txt"))
}

impl AppConfig {
    pub fn load() -> Self {
        let mut cfg = Self::default();
        let Ok(content) = fs::read_to_string(config_path()) else {
            return cfg;
        };

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Some((k, v)) = line.split_once('=') {
                match k.trim() {
                    "lang" => {
                        let v = v.trim();
                        cfg.lang = if v.is_empty() || v == "auto" {
                            None
                        } else {
                            Some(v.to_string())
                        };
                    }
                    "hotkey" => {
                        if let Some(hk) = HotkeyConfig::parse(v.trim()) {
                            cfg.hotkey = hk;
                        }
                    }
                    _ => {}
                }
            } else if line != "auto" {
                // Backward compatibility with single-line config.txt
                cfg.lang = Some(line.to_string());
            }
        }

        cfg
    }

    pub fn save(&self) {
        let lang_val = self.lang.as_deref().unwrap_or("auto");
        let content = format!("lang={lang_val}\nhotkey={}\n", self.hotkey.serialize());
        let _ = fs::write(config_path(), content);
    }
}
