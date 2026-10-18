//! 备忘录: notes with optional reminders, kept in `memos.json` next to the config,
//! and the reader for times written the way people say them ("明天下午3点",
//! "10分钟后", "周五 14:30").

use std::{fs, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::Config;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Repeat {
    #[default]
    None,
    Daily,
    Weekdays,
    Weekly,
}

impl Repeat {
    pub const ALL: [Repeat; 4] = [Repeat::None, Repeat::Daily, Repeat::Weekdays, Repeat::Weekly];

    pub fn label(self) -> &'static str {
        match self {
            Repeat::None => "不重复",
            Repeat::Daily => "每天",
            Repeat::Weekdays => "工作日",
            Repeat::Weekly => "每周",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Memo {
    pub id: String,
    pub text: String,
    /// Unix seconds.
    pub created: u64,
    pub updated: u64,
    pub pinned: bool,
    pub done: bool,
    /// When to remind (next), unix seconds.
    pub remind_at: Option<u64>,
    /// The reminder went off and won't again (unless snoozed).
    pub fired: bool,
    pub repeat: Repeat,
    /// 稍后提醒: an extra reminder at this time, besides the regular one.
    pub snoozed: Option<u64>,
}

impl Memo {
    /// The first line, for lists.
    pub fn title(&self) -> String {
        let line = self.text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("（空白）");
        line.chars().take(80).collect()
    }

    /// When it reminds next, if it still will.
    pub fn next_time(&self) -> Option<u64> {
        if self.done {
            return None;
        }
        let regular = self.remind_at.filter(|_| !self.fired);
        match (self.snoozed, regular) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    pub fn due(&self, now: u64) -> bool {
        self.next_time().is_some_and(|at| at <= now)
    }

    /// Its reminder went off: a snooze is used up, else the regular reminder moves on
    /// (repeating) or is done with. `weekday` gives the local weekday (0 Monday) of a time.
    pub fn fire(&mut self, now: u64, weekday: impl Fn(u64) -> u8) {
        if self.snoozed.is_some_and(|t| t <= now) {
            self.snoozed = None;
        }
        if !self.fired && self.remind_at.is_some_and(|t| t <= now) {
            after_fired(self, now, weekday);
        }
    }

    pub fn matches(&self, filter: &str) -> bool {
        let text = self.text.to_lowercase();
        filter.split_whitespace().all(|w| text.contains(&w.to_lowercase()))
    }
}

fn path() -> PathBuf {
    Config::dir().join("memos.json")
}

pub fn load() -> Vec<Memo> {
    fs::read_to_string(path())
        .ok()
        .and_then(|text| serde_json::from_str(text.trim_start_matches('\u{feff}')).ok())
        .unwrap_or_default()
}

pub fn save(list: &[Memo]) {
    let dir = Config::dir();
    let _ = fs::create_dir_all(&dir);
    let Ok(json) = serde_json::to_string_pretty(list) else { return };
    // Write-then-rename so a crash never leaves a truncated file behind.
    let tmp = dir.join("memos.json.tmp");
    if fs::write(&tmp, json).is_ok() {
        let _ = fs::rename(&tmp, path());
    }
}

/// List order: open ones first (pinned, then the soonest reminder, then newest), done last.
pub fn sort(list: &mut [Memo]) {
    list.sort_by_key(|m| (m.done, !m.pinned, m.next_time().unwrap_or(u64::MAX), std::cmp::Reverse(m.updated)));
}

/// After its reminder went off: a repeating memo moves on to its next time, any
/// other one is marked as fired. `weekday` gives the local weekday (0 Monday) of a time.
pub fn after_fired(memo: &mut Memo, now: u64, weekday: impl Fn(u64) -> u8) {
    let Some(mut at) = memo.remind_at else { return };
    if memo.repeat == Repeat::None {
        memo.fired = true;
        return;
    }
    const DAY: u64 = 86_400;
    // Repeat until it lies ahead (reminders missed while switched off are skipped).
    loop {
        at += match memo.repeat {
            Repeat::Weekly => 7 * DAY,
            _ => DAY,
        };
        let workday = weekday(at) < 5;
        if at > now && (memo.repeat != Repeat::Weekdays || workday) {
            break;
        }
    }
    memo.remind_at = Some(at);
    memo.fired = false;
}

/// When a reminder is, the way a person would say it: "今天 15:00", "明天 09:00",
/// "10月20日 09:00"; with "（N 分钟后）" when it is close.
pub fn when_label(at: u64, now: u64) -> String {
    let (Some(t), Some(n)) = (crate::win::local_time(at), crate::win::local_time(now)) else {
        return String::new();
    };
    let days = days_from_civil(t.0 as i32, t.1 as u32, t.2 as u32) - days_from_civil(n.0 as i32, n.1 as u32, n.2 as u32);
    let clock = format!("{:02}:{:02}", t.3, t.4);
    let mut label = match days {
        -1 => format!("昨天 {clock}"),
        0 => format!("今天 {clock}"),
        1 => format!("明天 {clock}"),
        2 => format!("后天 {clock}"),
        _ if t.0 == n.0 => format!("{}月{}日 {clock}", t.1, t.2),
        _ => format!("{}年{}月{}日 {clock}", t.0, t.1, t.2),
    };
    if at > now && at - now < 3600 {
        label.push_str(&format!("（{} 分钟后）", (at - now).div_ceil(60)));
    }
    label
}

/// Now, in local time, for [`parse_when`].
pub fn local_now() -> Now {
    let (y, mo, d, h, mi) = crate::win::local_time(crate::config::now_secs()).unwrap_or((1970, 1, 1, 0, 0));
    Now { y: y as i32, mo: mo as u32, d: d as u32, h: h as u32, mi: mi as u32 }
}

/// Local weekday of a unix time, 0 Monday.
pub fn local_weekday(ts: u64) -> u8 {
    crate::win::local_time(ts).map(|(y, m, d, _, _)| weekday(y as i32, m as u32, d as u32)).unwrap_or(0)
}

/// The unix time a parsed time stands for.
pub fn resolve(when: When) -> Option<u64> {
    match when {
        When::After(minutes) => Some(crate::config::now_secs() + minutes * 60),
        When::At { y, mo, d, h, mi } => crate::win::local_to_unix(y, mo, d, h, mi),
    }
}

// ---------------------------------------------------------------------------
// Reading times

/// The current local time, for [`parse_when`].
#[derive(Clone, Copy, Debug)]
pub struct Now {
    pub y: i32,
    pub mo: u32,
    pub d: u32,
    pub h: u32,
    pub mi: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum When {
    /// In so many minutes.
    After(u64),
    /// At a local date and time.
    At { y: i32, mo: u32, d: u32, h: u32, mi: u32 },
}

/// Days since 1970-01-01 of a civil date.
fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y } as i64;
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = m as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    ((if m <= 2 { y + 1 } else { y }) as i32, m, d)
}

/// Weekday of a civil date, 0 = Monday.
pub fn weekday(y: i32, m: u32, d: u32) -> u8 {
    // 1970-01-01 was a Thursday.
    ((days_from_civil(y, m, d) + 3).rem_euclid(7)) as u8
}

fn add_days((y, m, d): (i32, u32, u32), n: i64) -> (i32, u32, u32) {
    civil_from_days(days_from_civil(y, m, d) + n)
}

struct Scan {
    chars: Vec<char>,
    pos: usize,
}

impl Scan {
    fn rest(&self) -> String {
        self.chars[self.pos..].iter().collect()
    }

    fn eat(&mut self, words: &[&str]) -> Option<usize> {
        let rest = self.rest();
        let (ix, word) = words.iter().enumerate().find(|(_, w)| rest.starts_with(**w))?;
        self.pos += word.chars().count();
        Some(ix)
    }

    fn skip_spaces(&mut self) {
        while self.chars.get(self.pos).is_some_and(|c| c.is_whitespace() || matches!(c, ',' | '，' | '、')) {
            self.pos += 1;
        }
    }

    /// A number in digits or Chinese numerals (0–99), or 半 when `half`.
    fn number(&mut self) -> Option<u32> {
        let start = self.pos;
        let digits: String = self.chars[self.pos..].iter().take_while(|c| c.is_ascii_digit()).collect();
        if !digits.is_empty() {
            self.pos += digits.len();
            return digits.parse().ok();
        }
        let cn: Vec<char> = self.chars[self.pos..]
            .iter()
            .take_while(|c| "零一二两三四五六七八九十".contains(**c))
            .copied()
            .collect();
        let digit = |c: char| "零一二三四五六七八九".find(c).map(|i| (i / 3) as u32).or((c == '两').then_some(2));
        let value = match cn.as_slice() {
            [] => None,
            ['十'] => Some(10),
            ['十', b] => digit(*b).map(|b| 10 + b),
            [a, '十'] => digit(*a).map(|a| a * 10),
            [a, '十', b] => digit(*a).zip(digit(*b)).map(|(a, b)| a * 10 + b),
            [a] => digit(*a),
            _ => None,
        };
        match value {
            Some(v) => {
                self.pos += cn.len();
                Some(v)
            }
            None => {
                self.pos = start;
                None
            }
        }
    }
}

/// Read a time at the start of `input` ("明天下午3点 开会", "10分钟后喝水").
/// Returns it and the rest of the text, or `None` when there is no time.
pub fn parse_when(input: &str, now: &Now) -> Option<(When, String)> {
    let mut s = Scan { chars: input.trim().chars().collect(), pos: 0 };
    s.eat(&["提醒我", "提醒", "记得"]);
    s.skip_spaces();
    let start = s.pos;
    let when = relative(&mut s).or_else(|| {
        s.pos = start;
        absolute(&mut s, now)
    })?;
    s.skip_spaces();
    s.eat(&["提醒我", "提醒"]);
    s.skip_spaces();
    let rest = s.rest().trim_start_matches([':', '：', '-', ' ']).trim().to_string();
    Some((when, rest))
}

/// "10分钟后", "半小时后", "2小时30分钟后", "1天后", "10min", "2h".
fn relative(s: &mut Scan) -> Option<When> {
    let mut minutes = 0u64;
    let mut any = false;
    let mut ascii_unit = false;
    loop {
        let mark = s.pos;
        let n = if s.eat(&["半"]).is_some() { Some(None) } else { s.number().map(Some) };
        let Some(n) = n else { break };
        s.eat(&["个"]);
        let half_after = s.eat(&["半"]).is_some();
        let unit = s.eat(&["分钟", "分", "小时", "钟头", "天", "min", "m", "hours", "hour", "h", "d"]);
        let Some(unit) = unit else {
            s.pos = mark;
            break;
        };
        let per = match unit {
            0 | 1 | 5 | 6 => 1,
            2 | 3 | 7 | 8 | 9 => 60,
            _ => 1440,
        };
        ascii_unit |= unit >= 5;
        let value = match n {
            None => per / 2,
            Some(n) => n as u64 * per + if half_after { per / 2 } else { 0 },
        };
        minutes += value;
        any = true;
        s.skip_spaces();
    }
    if !any || minutes == 0 {
        return None;
    }
    if s.eat(&["以后", "之后", "后"]).is_none() && !ascii_unit {
        return None;
    }
    Some(When::After(minutes))
}

#[derive(Clone, Copy, PartialEq)]
enum Period {
    Dawn,
    Morning,
    Noon,
    Afternoon,
    Evening,
}

fn absolute(s: &mut Scan, now: &Now) -> Option<When> {
    let today = (now.y, now.mo, now.d);
    let date: Option<(i32, u32, u32)>;
    // A weekday that is today: next week's once the time has passed.
    let mut this_week_day = false;
    let mut period: Option<Period> = None;
    // Day words, some with a part of the day.
    if let Some(ix) = s.eat(&["大后天", "后天", "今天", "今日", "明天", "明日", "今晚", "明早", "明晨", "明晚", "今早"]) {
        let (days, p) = match ix {
            0 => (3, None),
            1 => (2, None),
            2 | 3 => (0, None),
            4 | 5 => (1, None),
            6 => (0, Some(Period::Evening)),
            7 | 8 => (1, Some(Period::Morning)),
            9 => (1, Some(Period::Evening)),
            _ => (0, Some(Period::Morning)),
        };
        date = Some(add_days(today, days));
        period = p;
    } else if let Some(next_week) = s.eat(&["下周", "下星期", "下礼拜", "周", "星期", "礼拜"]).map(|ix| ix < 3) {
        let ix = s.eat(&["一", "二", "三", "四", "五", "六", "日", "天", "末", "1", "2", "3", "4", "5", "6", "7"])?;
        let target = match ix {
            0..=6 => ix,
            7 => 6,
            8 => 5,
            _ => ix - 9,
        } as i64;
        let current = weekday(now.y, now.mo, now.d) as i64;
        let ahead = if next_week { 7 - current + target } else { (target - current).rem_euclid(7) };
        date = Some(add_days(today, ahead));
        this_week_day = !next_week;
    } else {
        date = calendar_date(s, now);
    }
    s.skip_spaces();
    if let Some(ix) = s.eat(&["凌晨", "早上", "早晨", "上午", "早", "中午", "下午", "傍晚", "晚上", "晚"]) {
        period = Some(match ix {
            0 => Period::Dawn,
            1..=4 => Period::Morning,
            5 => Period::Noon,
            6 => Period::Afternoon,
            _ => Period::Evening,
        });
    }
    s.skip_spaces();
    let time = clock(s);
    if date.is_none() && period.is_none() && time.is_none() {
        return None;
    }
    let colon = time.is_some_and(|t| t.2);
    let (mut h, mi) = time.map(|t| (t.0, t.1)).unwrap_or(match period {
        Some(Period::Dawn) => (6, 0),
        Some(Period::Noon) => (12, 0),
        Some(Period::Afternoon) => (15, 0),
        Some(Period::Evening) => (20, 0),
        _ => (9, 0),
    });
    match period {
        Some(Period::Afternoon | Period::Evening) if h < 12 => h += 12,
        Some(Period::Noon) if h < 11 => h += 12,
        _ => {}
    }
    if h > 23 || mi > 59 {
        return None;
    }
    let date = match date {
        Some(d) => d,
        None => {
            // Only a time: today if still ahead. "3点" said in the afternoon means 15:00.
            let passed = |h: u32| (h, mi) <= (now.h, now.mi);
            if !passed(h) {
                today
            } else if period.is_none() && !colon && (1..12).contains(&h) && !passed(h + 12) {
                h += 12;
                today
            } else {
                add_days(today, 1)
            }
        }
    };
    let date = if this_week_day && date == today && (h, mi) <= (now.h, now.mi) { add_days(date, 7) } else { date };
    Some(When::At { y: date.0, mo: date.1, d: date.2, h, mi })
}

/// "2026-10-20", "10-20", "10/20", "10月20日", "20号".
fn calendar_date(s: &mut Scan, now: &Now) -> Option<(i32, u32, u32)> {
    let start = s.pos;
    let Some(a) = s.number() else {
        s.pos = start;
        return None;
    };
    // A year first.
    let (year, first) = if a >= 1000 {
        if s.eat(&["-", "/", "年", "."]).is_none() {
            s.pos = start;
            return None;
        }
        (Some(a as i32), s.number())
    } else {
        (None, Some(a))
    };
    let Some(first) = first else {
        s.pos = start;
        return None;
    };
    let (month, day) = if s.eat(&["-", "/", "月", "."]).is_some() {
        let Some(day) = s.number() else {
            s.pos = start;
            return None;
        };
        s.eat(&["日", "号"]);
        (first, day)
    } else if year.is_none() && s.eat(&["号", "日"]).is_some() {
        // Just a day of the month: this month, or next if it has passed.
        let mut month = (now.y, now.mo);
        if first < now.d {
            month = if now.mo == 12 { (now.y + 1, 1) } else { (now.y, now.mo + 1) };
        }
        return valid(month.0, month.1, first);
    } else {
        s.pos = start;
        return None;
    };
    let y = year.unwrap_or_else(|| {
        // Without a year: this year, or next if the date has passed.
        if (month, day) < (now.mo, now.d) { now.y + 1 } else { now.y }
    });
    match valid(y, month, day) {
        Some(d) => Some(d),
        None => {
            s.pos = start;
            None
        }
    }
}

fn valid(y: i32, m: u32, d: u32) -> Option<(i32, u32, u32)> {
    ((1..=12).contains(&m) && d >= 1 && civil_from_days(days_from_civil(y, m, d)) == (y, m, d)).then_some((y, m, d))
}

/// "9点", "9点半", "9点30", "9点30分", "9:30", "21：00", "三点一刻": (hour, minute,
/// written with a colon, i.e. on the 24-hour clock).
fn clock(s: &mut Scan) -> Option<(u32, u32, bool)> {
    let start = s.pos;
    let Some(h) = s.number() else { return None };
    if s.eat(&[":", "："]).is_some() {
        let mark = s.pos;
        let digits: String = s.chars[s.pos..].iter().take_while(|c| c.is_ascii_digit()).collect();
        if digits.len() == 2 {
            s.pos += 2;
            return Some((h, digits.parse().ok()?, true));
        }
        s.pos = mark;
        s.pos = start;
        return None;
    }
    if s.eat(&["点", "时"]).is_none() {
        s.pos = start;
        return None;
    }
    if s.eat(&["半"]).is_some() {
        return Some((h, 30, false));
    }
    if s.eat(&["一刻"]).is_some() {
        return Some((h, 15, false));
    }
    if s.eat(&["三刻"]).is_some() {
        return Some((h, 45, false));
    }
    let mark = s.pos;
    match s.number() {
        Some(m) if m < 60 => {
            s.eat(&["分"]);
            Some((h, m, false))
        }
        _ => {
            s.pos = mark;
            s.eat(&["整"]);
            Some((h, 0, false))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Friday 2026-10-16, 14:20.
    const NOW: Now = Now { y: 2026, mo: 10, d: 16, h: 14, mi: 20 };

    fn at(input: &str) -> (When, String) {
        parse_when(input, &NOW).unwrap_or_else(|| panic!("no time in {input}"))
    }

    fn at_(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> When {
        When::At { y, mo, d, h, mi }
    }

    #[test]
    fn calendar() {
        assert_eq!(weekday(2026, 10, 16), 4);
        assert_eq!(weekday(1970, 1, 1), 3);
        assert_eq!(add_days((2026, 12, 31), 1), (2027, 1, 1));
        assert_eq!(add_days((2028, 2, 28), 1), (2028, 2, 29));
        assert_eq!(valid(2026, 2, 30), None);
    }

    #[test]
    fn relative_times() {
        assert_eq!(at("10分钟后 喝水"), (When::After(10), "喝水".into()));
        assert_eq!(at("半小时后开会"), (When::After(30), "开会".into()));
        assert_eq!(at("一个半小时后 出门").0, When::After(90));
        assert_eq!(at("2小时30分钟后 关火").0, When::After(150));
        assert_eq!(at("3天后 交报告").0, When::After(3 * 1440));
        assert_eq!(at("提醒我20分钟后喝水"), (When::After(20), "喝水".into()));
        assert_eq!(at("15min 休息").0, When::After(15));
        assert_eq!(at("2h 下班").0, When::After(120));
        assert!(parse_when("10个苹果", &NOW).is_none());
    }

    #[test]
    fn days_and_times() {
        assert_eq!(at("明天9点 开会"), (at_(2026, 10, 17, 9, 0), "开会".into()));
        assert_eq!(at("明天下午3点半开会").0, at_(2026, 10, 17, 15, 30));
        assert_eq!(at("明天下午三点一刻").0, at_(2026, 10, 17, 15, 15));
        assert_eq!(at("今晚8点 看电影").0, at_(2026, 10, 16, 20, 0));
        assert_eq!(at("今晚 倒垃圾"), (at_(2026, 10, 16, 20, 0), "倒垃圾".into()));
        assert_eq!(at("明早 跑步").0, at_(2026, 10, 17, 9, 0));
        assert_eq!(at("后天 交房租").0, at_(2026, 10, 18, 9, 0));
        assert_eq!(at("中午1点 吃饭").0, at_(2026, 10, 17, 13, 0), "13:00 has passed");
        assert_eq!(at("晚上十点半 睡觉").0, at_(2026, 10, 16, 22, 30));
        assert_eq!(at("21:30 洗衣服").0, at_(2026, 10, 16, 21, 30));
        assert_eq!(at("21：30 洗衣服").0, at_(2026, 10, 16, 21, 30));
        // Passed today: tomorrow. "3点" in the afternoon: 15:00.
        assert_eq!(at("10:00 周会").0, at_(2026, 10, 17, 10, 0));
        assert_eq!(at("3点 开会").0, at_(2026, 10, 16, 15, 0));
        assert_eq!(at("下午2点 开会").0, at_(2026, 10, 17, 14, 0));
    }

    #[test]
    fn weekdays_and_dates() {
        // Friday the 16th.
        assert_eq!(at("周五 14:30 下班").0, at_(2026, 10, 16, 14, 30));
        assert_eq!(at("周一 例会").0, at_(2026, 10, 19, 9, 0));
        assert_eq!(at("周五 9点 例会").0, at_(2026, 10, 23, 9, 0), "this Friday's 9:00 has passed");
        assert_eq!(at("周末 大扫除").0, at_(2026, 10, 17, 9, 0));
        assert_eq!(at("明天 时间管理培训"), (at_(2026, 10, 17, 9, 0), "时间管理培训".into()));
        assert_eq!(at("星期日下午 爬山").0, at_(2026, 10, 18, 15, 0));
        assert_eq!(at("下周三 10点 评审").0, at_(2026, 10, 21, 10, 0));
        assert_eq!(at("10月20日 体检").0, at_(2026, 10, 20, 9, 0));
        assert_eq!(at("10-20 9:30 体检").0, at_(2026, 10, 20, 9, 30));
        assert_eq!(at("1月5号 年会").0, at_(2027, 1, 5, 9, 0));
        assert_eq!(at("2026-12-25 圣诞").0, at_(2026, 12, 25, 9, 0));
        assert_eq!(at("20号 还信用卡").0, at_(2026, 10, 20, 9, 0));
        assert_eq!(at("5号 还信用卡").0, at_(2026, 11, 5, 9, 0));
    }

    #[test]
    fn no_time() {
        assert!(parse_when("买牛奶", &NOW).is_none());
        assert!(parse_when("", &NOW).is_none());
        assert!(parse_when("2026 计划", &NOW).is_none());
    }

    #[test]
    fn repeating_reminders_move_on() {
        // 2026-10-16 (Friday) 09:00 local, treating unix days as local days.
        let day = 86_400;
        let friday = days_from_civil(2026, 10, 16) as u64 * day + 9 * 3600;
        let wd = |t: u64| weekday_of(t);
        let mut memo = Memo { remind_at: Some(friday), repeat: Repeat::Weekdays, ..Default::default() };
        after_fired(&mut memo, friday, wd);
        assert_eq!(memo.remind_at, Some(friday + 3 * day), "Friday → Monday");
        memo.repeat = Repeat::Daily;
        after_fired(&mut memo, friday + 10 * day, wd);
        assert_eq!(memo.remind_at, Some(friday + 11 * day), "missed days are skipped");
        memo.repeat = Repeat::None;
        after_fired(&mut memo, friday + 20 * day, wd);
        assert!(memo.fired);
    }

    #[test]
    fn snoozing() {
        let wd = |t: u64| weekday_of(t);
        let mut memo = Memo { remind_at: Some(1000), ..Default::default() };
        assert!(memo.due(1000));
        memo.fire(1000, wd);
        assert!(memo.fired && !memo.due(5000));
        memo.snoozed = Some(1600);
        assert!(!memo.due(1599) && memo.due(1600));
        assert_eq!(memo.next_time(), Some(1600));
        memo.fire(1600, wd);
        assert_eq!(memo.next_time(), None);
        memo.snoozed = Some(2000);
        memo.done = true;
        assert!(!memo.due(3000), "done ones stay quiet");
    }

    fn weekday_of(t: u64) -> u8 {
        let (y, m, d) = civil_from_days((t / 86_400) as i64);
        weekday(y, m, d)
    }

    #[test]
    fn order() {
        let memo = |id: &str, pinned: bool, done: bool, at: Option<u64>, updated: u64| Memo {
            id: id.into(),
            pinned,
            done,
            remind_at: at,
            updated,
            ..Default::default()
        };
        let mut list = vec![
            memo("old", false, false, None, 1),
            memo("done", true, true, None, 9),
            memo("later", false, false, Some(200), 2),
            memo("soon", false, false, Some(100), 3),
            memo("pin", true, false, None, 4),
            memo("new", false, false, None, 5),
        ];
        sort(&mut list);
        let ids: Vec<_> = list.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["pin", "soon", "later", "new", "old", "done"]);
    }
}
