use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::net::HEARTHSTONE_EXE;
use crate::process::{get_pid_by_name, process_path};

const POWER_SECTION: &str = "[Power]\nLogLevel=1\nFilePrinting=true\nConsolePrinting=false\nScreenPrinting=false\nVerbose=true\n";
const LOG_SIZE_LIMIT_KB: &str = "500000";
const TRUNCATED_MARKER: &str = "Truncating log, which has reached the size limit";
const PREFIX: &str = "GameState.DebugPrintPower() - ";
const POLL: Duration = Duration::from_millis(100);
const RESCAN: Duration = Duration::from_secs(2);

static STATE: LazyLock<Mutex<GameState>> = LazyLock::new(|| Mutex::new(GameState::new()));
static AVAILABLE: AtomicBool = AtomicBool::new(false);
static TRUNCATED: AtomicBool = AtomicBool::new(false);

pub fn ensure_log_config() {
    let Some(dir) = std::env::var_os("LOCALAPPDATA")
        .map(|d| PathBuf::from(d).join("Blizzard").join("Hearthstone"))
    else {
        return;
    };
    let path = dir.join("log.config");
    let original = fs::read_to_string(&path).unwrap_or_default();
    let mut config = original.clone();
    if !has_section(&config, "Power") {
        append(&mut config, POWER_SECTION);
    }
    ensure_key(&mut config, "Log", "FileSizeLimit.Int", LOG_SIZE_LIMIT_KB);
    if config != original {
        let _ = fs::create_dir_all(&dir);
        let _ = fs::write(path, config);
    }
}

fn is_header(line: &str, section: &str) -> bool {
    line.trim()
        .strip_prefix('[')
        .and_then(|l| l.strip_suffix(']'))
        .is_some_and(|name| name.eq_ignore_ascii_case(section))
}

fn has_section(config: &str, section: &str) -> bool {
    config.lines().any(|l| is_header(l, section))
}

fn append(config: &mut String, text: &str) {
    if !config.is_empty() && !config.ends_with('\n') {
        config.push('\n');
    }
    *config += text;
}

fn ensure_key(config: &mut String, section: &str, key: &str, value: &str) {
    let lines: Vec<&str> = config.lines().collect();
    let Some(header) = lines.iter().position(|l| is_header(l, section)) else {
        append(config, &format!("[{section}]\n{key}={value}\n"));
        return;
    };
    let has_key = lines[header + 1..]
        .iter()
        .take_while(|l| !l.trim_start().starts_with('['))
        .any(|l| {
            l.split_once('=')
                .is_some_and(|(k, _)| k.trim().eq_ignore_ascii_case(key))
        });
    if has_key {
        return;
    }
    let mut out: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
    out.insert(header + 1, format!("{key}={value}"));
    *config = out.join("\n") + "\n";
}

pub fn available() -> bool {
    AVAILABLE.load(Ordering::Relaxed) && !TRUNCATED.load(Ordering::Relaxed)
}

pub fn board_count() -> usize {
    STATE.lock().unwrap().board_count()
}

pub fn played_seq() -> u64 {
    STATE.lock().unwrap().played_seq
}

pub fn last_played() -> Option<u32> {
    STATE.lock().unwrap().last_played
}

pub fn slot_of(id: u32) -> Option<(u32, usize)> {
    let state = STATE.lock().unwrap();
    let e = state.entities.get(&id)?;
    (e.zone == "PLAY" && e.zone_pos > 0).then(|| (e.zone_pos, state.board_count()))
}

pub fn left_seq() -> u64 {
    STATE.lock().unwrap().left_seq
}

pub fn last_left() -> Option<u32> {
    STATE.lock().unwrap().last_left
}

pub fn start() {
    ensure_log_config();
    thread::spawn(|| {
        let mut path: Option<PathBuf> = None;
        let mut offset = 0u64;
        let mut partial = Vec::new();
        let mut last_scan = Instant::now() - RESCAN;
        loop {
            if last_scan.elapsed() >= RESCAN {
                last_scan = Instant::now();
                let found = get_pid_by_name(HEARTHSTONE_EXE).and_then(find_power_log);
                if found != path {
                    path = found;
                    offset = 0;
                    partial.clear();
                    TRUNCATED.store(false, Ordering::Relaxed);
                    *STATE.lock().unwrap() = GameState::new();
                }
                AVAILABLE.store(path.is_some(), Ordering::Relaxed);
            }
            if let Some(p) = &path {
                read_new_lines(p, &mut offset, &mut partial);
            }
            thread::sleep(POLL);
        }
    });
}

fn find_power_log(pid: u32) -> Option<PathBuf> {
    let logs = process_path(pid)?.parent()?.join("Logs");
    let newest_session = fs::read_dir(&logs)
        .ok()?
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().starts_with("Hearthstone_"))
        .map(|e| e.path())
        .max();
    let candidate = match newest_session {
        Some(dir) => dir.join("Power.log"),
        None => logs.join("Power.log"),
    };
    candidate.is_file().then_some(candidate)
}

fn read_new_lines(path: &Path, offset: &mut u64, partial: &mut Vec<u8>) {
    let Ok(mut file) = File::open(path) else {
        return;
    };
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    if len < *offset {
        // Файл пересоздан — читаем заново
        *offset = 0;
        partial.clear();
        *STATE.lock().unwrap() = GameState::new();
    }
    if len == *offset || file.seek(SeekFrom::Start(*offset)).is_err() {
        return;
    }
    let mut buf = Vec::new();
    let Ok(read) = file.read_to_end(&mut buf) else {
        return;
    };
    *offset += read as u64;
    partial.extend_from_slice(&buf);

    let Some(last_newline) = partial.iter().rposition(|&b| b == b'\n') else {
        return;
    };
    let complete: Vec<u8> = partial.drain(..=last_newline).collect();
    let mut state = STATE.lock().unwrap();
    for line in String::from_utf8_lossy(&complete).lines() {
        if line.contains(TRUNCATED_MARKER) {
            TRUNCATED.store(true, Ordering::Relaxed);
        }
        state.line(line);
    }
}

#[derive(Clone, Default)]
struct Entity {
    controller: u32,
    zone: String,
    zone_pos: u32,
    cardtype: String,
}

struct GameState {
    entities: HashMap<u32, Entity>,
    local: Option<u32>,
    current: Option<u32>,
    played_seq: u64,
    last_played: Option<u32>,
    left_seq: u64,
    last_left: Option<u32>,
}

impl GameState {
    fn new() -> Self {
        Self {
            entities: HashMap::new(),
            local: None,
            current: None,
            played_seq: 0,
            last_played: None,
            left_seq: 0,
            last_left: None,
        }
    }

    fn board_count(&self) -> usize {
        let Some(local) = self.local else {
            return 0;
        };
        self.entities
            .values()
            .filter(|e| e.controller == local && e.zone == "PLAY" && e.cardtype == "MINION")
            .count()
    }

    fn line(&mut self, line: &str) {
        let Some(idx) = line.find(PREFIX) else {
            return;
        };
        let body = line[idx + PREFIX.len()..].trim();

        if let Some(rest) = body.strip_prefix("tag=") {
            if let (Some(id), Some((tag, value))) = (self.current, rest.split_once(" value=")) {
                self.set(id, tag.trim(), value.trim(), true);
            }
            return;
        }
        self.current = None;

        if body.starts_with("CREATE_GAME") {
            let (played_seq, left_seq) = (self.played_seq, self.left_seq);
            *self = Self::new();
            self.played_seq = played_seq;
            self.left_seq = left_seq;
        } else if let Some(rest) = body.strip_prefix("Player EntityID=") {
            self.player(rest);
        } else if let Some(rest) = body.strip_prefix("FULL_ENTITY - Creating ID=") {
            self.current = leading_number(rest);
        } else if body.starts_with("FULL_ENTITY - Updating")
            || body.starts_with("SHOW_ENTITY - Updating")
            || body.starts_with("CHANGE_ENTITY - Updating")
        {
            self.current = entity_id(body);
        } else if let Some(rest) = body
            .strip_prefix("TAG_CHANGE Entity=")
            .or_else(|| body.strip_prefix("HIDE_ENTITY - Entity="))
        {
            let Some(tag_at) = rest.rfind(" tag=") else {
                return;
            };
            let (Some(id), Some((tag, value))) = (
                entity_id(&rest[..tag_at]),
                rest[tag_at + 5..].split_once(" value="),
            ) else {
                return;
            };
            let value = value.split_whitespace().next().unwrap_or("");
            self.set(id, tag.trim(), value, false);
        }
    }

    fn player(&mut self, rest: &str) {
        let player_id = rest
            .split_once("PlayerID=")
            .and_then(|(_, r)| leading_number(r));
        let lo = rest
            .split_once("lo=")
            .and_then(|(_, r)| r.trim_end_matches(']').trim().parse::<u64>().ok());
        if let (Some(id), Some(lo)) = (player_id, lo)
            && lo != 0
            && self.local.is_none()
        {
            self.local = Some(id);
        }
    }

    fn set(&mut self, id: u32, tag: &str, value: &str, in_block: bool) {
        let local = self.local;
        let e = self.entities.entry(id).or_default();
        match tag {
            "CONTROLLER" => e.controller = value.parse().unwrap_or(0),
            "CARDTYPE" => e.cardtype = value.to_string(),
            "ZONE_POSITION" => e.zone_pos = value.parse().unwrap_or(0),
            "ZONE" => {
                let from = std::mem::replace(&mut e.zone, value.to_string());
                let own_minion = e.cardtype == "MINION" && Some(e.controller) == local;
                if own_minion && from == "HAND" && value == "PLAY" {
                    self.played_seq += 1;
                    self.last_played = Some(id);
                }
                if own_minion && from == "PLAY" && value != "PLAY" {
                    self.left_seq += 1;
                    self.last_left = Some(id);
                }
                // Запасной способ узнать себя: известная карта, созданная прямо в руке
                if in_block && value == "HAND" && e.controller != 0 && self.local.is_none() {
                    self.local = Some(e.controller);
                }
            }
            _ => {}
        }
    }
}

fn leading_number(s: &str) -> Option<u32> {
    let end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    s[..end].parse().ok()
}

fn entity_id(s: &str) -> Option<u32> {
    match s.find(" id=") {
        Some(at) if s.contains('[') => leading_number(&s[at + 4..]),
        _ => leading_number(s.trim_start_matches("Entity=")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(state: &mut GameState, lines: &str) {
        for l in lines.lines() {
            state.line(&format!("D 12:00:00.0000000 {PREFIX}{}", l.trim_start()));
        }
    }

    fn minion(state: &mut GameState, id: u32, controller: u32, zone: &str, pos: u32) {
        feed(
            state,
            &format!(
                "FULL_ENTITY - Creating ID={id} CardID=BGS_004\n\
                 tag=CONTROLLER value={controller}\n\
                 tag=CARDTYPE value=MINION\n\
                 tag=ZONE value={zone}\n\
                 tag=ZONE_POSITION value={pos}"
            ),
        );
    }

    fn game() -> GameState {
        let mut s = GameState::new();
        feed(
            &mut s,
            "CREATE_GAME\n\
             Player EntityID=2 PlayerID=5 GameAccountId=[hi=144115193835963207 lo=12345678]\n\
             Player EntityID=3 PlayerID=13 GameAccountId=[hi=0 lo=0]",
        );
        s
    }

    #[test]
    fn detects_local_player() {
        assert_eq!(game().local, Some(5));
    }

    #[test]
    fn counts_only_own_minions_on_board() {
        let mut s = game();
        minion(&mut s, 70, 5, "PLAY", 1);
        minion(&mut s, 71, 5, "PLAY", 2);
        minion(&mut s, 80, 13, "PLAY", 1); // таверна Боба
        minion(&mut s, 72, 5, "HAND", 1);
        assert_eq!(s.board_count(), 2);
    }

    #[test]
    fn tracks_minion_played_from_hand() {
        let mut s = game();
        minion(&mut s, 70, 5, "PLAY", 1);
        minion(&mut s, 72, 5, "HAND", 1);
        feed(
            &mut s,
            "TAG_CHANGE Entity=[entityName=Wrath Weaver id=72 zone=HAND zonePos=1 cardId=BGS_004 player=5] tag=ZONE value=PLAY \n\
             TAG_CHANGE Entity=[entityName=Wrath Weaver id=72 zone=PLAY zonePos=0 cardId=BGS_004 player=5] tag=ZONE_POSITION value=2 ",
        );
        assert_eq!(s.played_seq, 1);
        assert_eq!(s.last_played, Some(72));
        let e = &s.entities[&72];
        assert_eq!((e.zone.as_str(), e.zone_pos), ("PLAY", 2));
        assert_eq!(s.board_count(), 2);
    }

    #[test]
    fn buying_from_bob_is_not_a_play() {
        let mut s = game();
        minion(&mut s, 80, 13, "PLAY", 1);
        feed(
            &mut s,
            "TAG_CHANGE Entity=[entityName=Alleycat id=80 zone=PLAY zonePos=1 cardId=BG_CFM_315 player=13] tag=CONTROLLER value=5 \n\
             TAG_CHANGE Entity=[entityName=Alleycat id=80 zone=PLAY zonePos=1 cardId=BG_CFM_315 player=5] tag=ZONE value=HAND ",
        );
        assert_eq!(s.played_seq, 0);
        assert_eq!(s.board_count(), 0);
    }

    #[test]
    fn detects_sale() {
        let mut s = game();
        minion(&mut s, 70, 5, "PLAY", 1);
        minion(&mut s, 71, 5, "PLAY", 2);
        feed(
            &mut s,
            "TAG_CHANGE Entity=[entityName=Foo id=70 zone=PLAY zonePos=1 cardId=X player=5] tag=ZONE_POSITION value=0 \n\
             TAG_CHANGE Entity=[entityName=Foo id=70 zone=PLAY zonePos=1 cardId=X player=5] tag=ZONE value=SETASIDE ",
        );
        assert_eq!((s.left_seq, s.last_left), (1, Some(70)));
        assert_eq!(s.board_count(), 1);
    }

    #[test]
    fn log_config_keeps_user_settings() {
        let mut c = "[Power]\nLogLevel=1\n[Log]\nOther=1\n".to_string();
        ensure_key(&mut c, "Log", "FileSizeLimit.Int", "500000");
        assert_eq!(
            c,
            "[Power]\nLogLevel=1\n[Log]\nFileSizeLimit.Int=500000\nOther=1\n"
        );
        // Повторный вызов ничего не меняет, свой лимит пользователя не перетираем
        let before = c.clone();
        ensure_key(&mut c, "Log", "FileSizeLimit.Int", "1");
        assert_eq!(c, before);

        let mut short = "[Power]\nLogLevel=1".to_string();
        ensure_key(&mut short, "Log", "FileSizeLimit.Int", "500000");
        assert_eq!(
            short,
            "[Power]\nLogLevel=1\n[Log]\nFileSizeLimit.Int=500000\n"
        );
    }

    #[test]
    fn ignores_power_task_list_duplicates() {
        let mut s = game();
        s.line(
            "D 12:00:00.0 PowerTaskList.DebugPrintPower() - FULL_ENTITY - Creating ID=70 CardID=X",
        );
        s.line("D 12:00:00.0 PowerTaskList.DebugPrintPower() -     tag=ZONE value=PLAY");
        assert!(s.entities.is_empty());
    }

    #[test]
    fn parses_entity_ids() {
        assert_eq!(
            entity_id("[entityName=Foo id=77 zone=HAND zonePos=1 cardId=X player=5]"),
            Some(77)
        );
        assert_eq!(entity_id("77"), Some(77));
        assert_eq!(entity_id("GameEntity"), None);
    }
}

#[cfg(test)]
mod real_log_probe {
    use super::*;

    #[test]
    #[ignore]
    fn probe() {
        let path = std::env::var("POWER_LOG").unwrap();
        let text = fs::read_to_string(path).unwrap();
        let mut s = GameState::new();
        let mut last_seq = 0;
        for line in text.lines() {
            s.line(line);
            if s.played_seq != last_seq {
                last_seq = s.played_seq;
                let e = &s.entities[&s.last_played.unwrap()];
                println!(
                    "{} played id={} pos={} board={} local={:?}",
                    &line[2..16],
                    s.last_played.unwrap(),
                    e.zone_pos,
                    s.board_count(),
                    s.local
                );
            }
        }
        println!(
            "END local={:?} board={} played={} left={}",
            s.local,
            s.board_count(),
            s.played_seq,
            s.left_seq
        );
    }
}
