use std::fs;
use std::path::{Path, PathBuf};

/// A Steam game that is currently running.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Game {
    pub app_id: u32,
    pub name: String,
    /// Steam's `reaper` process that launched the game.
    pub launcher_pid: u32,
}

impl Game {
    /// Directory-friendly identifier, e.g. `1363080-manor-lords`.
    pub fn slug(&self) -> String {
        let mut slug = String::new();
        for c in self.name.chars() {
            if c.is_alphanumeric() {
                slug.extend(c.to_lowercase());
            } else if !slug.ends_with('-') {
                slug.push('-');
            }
        }
        let slug = slug.trim_matches('-');
        if slug.is_empty() {
            self.app_id.to_string()
        } else {
            format!("{}-{}", self.app_id, slug)
        }
    }
}

/// Finds the running Steam game with id `app_id`.
///
/// Steam starts every game through its `reaper` process, whose command line
/// looks like `reaper SteamLaunch AppId=1363080 -- ...`, so scanning
/// `/proc/*/cmdline` for that marker works for native and Proton games alike.
pub fn running_game(app_id: u32) -> Option<Game> {
    let entries = fs::read_dir("/proc").ok()?;
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let Some(pid) = file_name.to_str() else {
            continue;
        };
        if !pid.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let Ok(cmdline) = fs::read(entry.path().join("cmdline")) else {
            continue;
        };
        if app_id_from_cmdline(&cmdline) != Some(app_id) {
            continue;
        }
        let name = game_name(app_id).unwrap_or_else(|| format!("app {app_id}"));
        return Some(Game {
            app_id,
            name,
            launcher_pid: pid.parse().ok()?,
        });
    }
    None
}

fn app_id_from_cmdline(cmdline: &[u8]) -> Option<u32> {
    let mut args = cmdline.split(|&b| b == 0);
    if !args.any(|a| a == b"SteamLaunch") {
        return None;
    }
    args.take_while(|a| *a != b"--")
        .find_map(|a| a.strip_prefix(b"AppId="))
        .and_then(|id| std::str::from_utf8(id).ok()?.parse().ok())
        .filter(|&id| id != 0)
}

fn steam_root() -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    [
        ".steam/steam",
        ".local/share/Steam",
        ".var/app/com.valvesoftware.Steam/data/Steam",
    ]
    .iter()
    .map(|p| home.join(p))
    .find(|p| p.join("steamapps").is_dir())
}

/// Every `steamapps` directory, read from `libraryfolders.vdf`.
fn library_dirs() -> Vec<PathBuf> {
    let Some(root) = steam_root() else {
        return Vec::new();
    };
    let mut dirs = vec![root.join("steamapps")];
    if let Ok(vdf) = fs::read_to_string(root.join("steamapps/libraryfolders.vdf")) {
        for path in vdf.lines().filter_map(|l| vdf_value(l, "path")) {
            let dir = Path::new(&path).join("steamapps");
            if !dirs.contains(&dir) {
                dirs.push(dir);
            }
        }
    }
    dirs
}

pub fn game_name(app_id: u32) -> Option<String> {
    library_dirs().iter().find_map(|dir| {
        let acf = fs::read_to_string(dir.join(format!("appmanifest_{app_id}.acf"))).ok()?;
        acf.lines().find_map(|l| vdf_value(l, "name"))
    })
}

/// Parses a `"key"		"value"` line from a Valve KeyValues file.
fn vdf_value(line: &str, key: &str) -> Option<String> {
    let mut parts = line.split('"').filter(|p| !p.trim().is_empty());
    (parts.next()? == key).then(|| parts.next().map(str::to_owned))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_reaper_cmdline() {
        let cmdline = b"/home/u/.steam/ubuntu12_32/reaper\0SteamLaunch\0AppId=1363080\0--\0/bin/game\0AppId=5\0";
        assert_eq!(app_id_from_cmdline(cmdline), Some(1363080));
        assert_eq!(app_id_from_cmdline(b"/bin/game\0AppId=5\0"), None);
    }

    #[test]
    fn parses_vdf_line() {
        assert_eq!(
            vdf_value("\t\"name\"\t\t\"Manor Lords\"", "name").as_deref(),
            Some("Manor Lords")
        );
        assert_eq!(vdf_value("\t\"appid\"\t\t\"1\"", "name"), None);
    }

    #[test]
    fn slugifies_name() {
        let game = Game {
            app_id: 42,
            name: "Disco Elysium - The Final Cut".into(),
            launcher_pid: 1,
        };
        assert_eq!(game.slug(), "42-disco-elysium-the-final-cut");
    }
}
