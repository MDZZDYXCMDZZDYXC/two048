//! # two048 — Pumpkin 服务器「命令对话版 2048」插件
//!
//! 参考 <https://docs.pumpkinmc.org/plugin-dev/rust> 的插件与命令开发指南实现。
//!
//! 玩家在聊天栏里通过 `/2048 ...` 系列子命令游玩 2048：
//! 每一步都会把当前棋盘、得分与可点击的方向按钮重新推送给玩家，
//! 形成「命令行对话式」的完整对局体验。
//!
//! ## 支持的命令
//!
//! | 命令 | 别名 | 说明 |
//! | --- | --- | --- |
//! | `/2048` | `t2048` `game2048` | 显示当前棋盘（没有对局时给出提示） |
//! | `/2048 start` | `new` `restart` `reset` | 开始 / 重开一局 |
//! | `/2048 up` | `w` | 向上移动 |
//! | `/2048 down` | `s` | 向下移动 |
//! | `/2048 left` | `a` | 向左移动 |
//! | `/2048 right` | `d` | 向右移动 |
//! | `/2048 undo` | `back` `z` | 撤销上一步 |
//! | `/2048 show` | `board` `b` | 重新显示棋盘 |
//! | `/2048 best` | `top` `rank` `leaderboard` | 全服排行榜（前 10 名 + 自己的名次） |
//! | `/2048 me` | `stats` `profile` | 我的战绩与排名 |
//! | `/2048 stop` | `quit` `end` `exit` | 结束当前对局 |
//! | `/2048 help` | `h` | 玩法说明 |
//!
//! ## 排行榜
//!
//! 每位玩家的战绩（最高分、最大方块、完成局数、累计步数、累计得分）会记录在
//! 插件数据目录下的 `leaderboard.tsv` 中，服务端重启后依然保留。
//! 排序规则：最高分 → 最大方块 → 完成局数 → 玩家名（字典序）。
//!
//! 榜单会在这些时机刷新：对局无路可走时、`/2048 stop` 时、以及用
//! `/2048 start` 放弃一局进行中的对局时（未产生任何步数的空局不计入）。

use std::sync::{Mutex, MutexGuard};

use pumpkin_plugin_api::command::{
    Command, CommandError, CommandNode, CommandSender, ConsumedArgs,
};
use pumpkin_plugin_api::commands::CommandHandler;
use pumpkin_plugin_api::common::NamedColor;
use pumpkin_plugin_api::permission::{Permission, PermissionDefault};
use pumpkin_plugin_api::permissions::{FS_READ_DATA, FS_WRITE_DATA};
use pumpkin_plugin_api::text::TextComponent;
use pumpkin_plugin_api::{Context, Plugin, PluginMetadata, Server};

/// 棋盘边长。
const SIZE: usize = 4;
/// 格子总数。
const CELLS: usize = SIZE * SIZE;
/// 达到该数字即为通关。
const TARGET: u32 = 2048;
/// 命令所需的权限节点。
const PERM_NODE: &str = "two048:command.play";
/// 每个玩家最多保留多少步可撤销历史。
const MAX_HISTORY: usize = 24;
/// 排行榜最多展示多少名。
const TOP_N: usize = 10;
/// 排行榜文件名（位于插件数据目录内）。
const BOARD_FILE: &str = "leaderboard.tsv";
/// 帮助里的命令条数。
const HELP_COUNT: usize = 11;

/// 命令描述。
///
/// 命令树在 `on_load` 里一次性注册，那时没有任何玩家上下文，因此这条**无法**
/// 按玩家语言渲染。它在客户端的补全提示里对玩家可见，所以两种语言并列写在同一行。
const COMMAND_DESCRIPTION: &str = "在聊天栏里玩 2048 小游戏 · Play 2048 in chat";
/// 插件描述，同样在 `on_load` 时注册，无法按玩家语言渲染。
const PLUGIN_DESCRIPTION: &str =
    "在聊天栏里用命令对话玩 2048 小游戏（含全服排行榜）· Command-driven 2048 in chat, with a server-wide leaderboard";

/// 帮助里的命令字面量，与各语言的 `help_descriptions` 一一对应。
///
/// 命令字面量本身不参与本地化：翻译它们会随玩家切换语言而悄悄改变命令语法。
const HELP_COMMANDS: [&str; HELP_COUNT] = [
    "2048 start",
    "2048 up",
    "2048 down",
    "2048 left",
    "2048 right",
    "2048 undo",
    "2048 show",
    "2048 best",
    "2048 me",
    "2048 stop",
    "2048 help",
];

// ---------------------------------------------------------------------------
// 全局状态
// ---------------------------------------------------------------------------

/// 所有在线的、正在进行的对局，键为玩家名。
static GAMES: Mutex<Vec<Entry>> = Mutex::new(Vec::new());
/// 全服排行榜：每位玩家一条累计战绩。
static PROFILES: Mutex<Vec<Profile>> = Mutex::new(Vec::new());
/// 排行榜文件的绝对路径，由 `on_load` 写入。
static DATA_FILE: Mutex<Option<String>> = Mutex::new(None);

/// 一个玩家名对应的对局。
struct Entry {
    owner: String,
    game: Game,
}

/// 一位玩家的累计战绩。
#[derive(Clone)]
struct Profile {
    name: String,
    /// 历史最高分。
    best_score: u32,
    /// 历史最大方块。
    best_tile: u32,
    /// 完成的（或中途放弃的）局数。
    games: u32,
    /// 累计步数。
    steps: u64,
    /// 累计得分，用于计算平均分。
    score_sum: u64,
}

fn games() -> MutexGuard<'static, Vec<Entry>> {
    GAMES.lock().unwrap_or_else(|e| e.into_inner())
}

fn profiles() -> MutexGuard<'static, Vec<Profile>> {
    PROFILES.lock().unwrap_or_else(|e| e.into_inner())
}

fn data_file() -> Option<String> {
    DATA_FILE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

fn index_of(list: &[Entry], owner: &str) -> Option<usize> {
    list.iter().position(|e| e.owner == owner)
}

/// 读取某位玩家当前的对局快照。
fn snapshot_of(owner: &str) -> Option<Game> {
    let list = games();
    index_of(&list, owner).map(|i| list[i].game.clone())
}

// ---------------------------------------------------------------------------
// 排行榜
// ---------------------------------------------------------------------------

/// 按榜单规则排好序的副本（最高分 → 最大方块 → 局数 → 玩家名）。
fn sorted_profiles() -> Vec<Profile> {
    let mut list = profiles().clone();
    list.sort_by(|a, b| {
        b.best_score
            .cmp(&a.best_score)
            .then(b.best_tile.cmp(&a.best_tile))
            .then(b.games.cmp(&a.games))
            .then(a.name.cmp(&b.name))
    });
    list
}

/// 某位玩家当前的名次（从 1 开始）；没有上榜返回 `None`。
fn rank_of(owner: &str) -> Option<usize> {
    sorted_profiles()
        .iter()
        .position(|p| p.name == owner)
        .map(|i| i + 1)
}

/// 一局结束后交给玩家看的排名结果。
struct GameResult {
    rank: Option<usize>,
    total: usize,
    new_best: bool,
}

/// 把一局的结果累加进排行榜并落盘。
fn finish_game(owner: &str, game: &Game) -> GameResult {
    let new_best;
    {
        let mut list = profiles();
        let previous = list
            .iter()
            .find(|p| p.name == owner)
            .map_or(0, |p| p.best_score);
        new_best = game.score > previous;

        match list.iter_mut().find(|p| p.name == owner) {
            Some(profile) => {
                profile.best_score = profile.best_score.max(game.score);
                profile.best_tile = profile.best_tile.max(game.max_tile());
                profile.games += 1;
                profile.steps += game.steps as u64;
                profile.score_sum += game.score as u64;
            }
            None => list.push(Profile {
                name: owner.to_string(),
                best_score: game.score,
                best_tile: game.max_tile(),
                games: 1,
                steps: game.steps as u64,
                score_sum: game.score as u64,
            }),
        }
    }

    save_profiles();

    let sorted = sorted_profiles();
    let total = sorted.len();
    let rank = sorted.iter().position(|p| p.name == owner).map(|i| i + 1);
    GameResult {
        rank,
        total,
        new_best,
    }
}

/// 从磁盘载入排行榜。文件不存在或损坏时静默跳过对应的行。
fn load_profiles(path: &str) {
    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };

    let mut loaded = Vec::new();
    for line in text.lines() {
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() < 6 || fields[0].is_empty() {
            continue;
        }
        if let (Ok(best_score), Ok(best_tile), Ok(games), Ok(steps), Ok(score_sum)) = (
            fields[1].parse::<u32>(),
            fields[2].parse::<u32>(),
            fields[3].parse::<u32>(),
            fields[4].parse::<u64>(),
            fields[5].parse::<u64>(),
        ) {
            loaded.push(Profile {
                name: fields[0].to_string(),
                best_score,
                best_tile,
                games,
                steps,
                score_sum,
            });
        }
    }

    *profiles() = loaded;
}

/// 把排行榜写回磁盘。失败时只记日志，不影响游戏。
fn save_profiles() {
    let Some(path) = data_file() else {
        return;
    };

    let mut out = String::new();
    {
        let list = profiles();
        for profile in list.iter() {
            out.push_str(&format!(
                "{}\t{}\t{}\t{}\t{}\t{}\n",
                profile.name,
                profile.best_score,
                profile.best_tile,
                profile.games,
                profile.steps,
                profile.score_sum
            ));
        }
    }

    if let Some(parent) = std::path::Path::new(&path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(err) = std::fs::write(&path, out) {
        tracing::warn!("two048: 排行榜写入 {path} 失败：{err}");
    }
}

// ---------------------------------------------------------------------------
// 伪随机数：uuid + 时间混合播种的 splitmix64
// ---------------------------------------------------------------------------

struct Rng {
    state: u64,
}

impl Rng {
    fn new() -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as u64);
        let uid = pumpkin_plugin_api::uuid::generate();
        Self {
            state: now ^ uid.high ^ uid.low.rotate_left(32) ^ 0x9E37_79B9_7F4A_7C15,
        }
    }

    /// splitmix64：每调用一次推进一步，返回质量良好的 64 位随机数。
    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// 返回 `[0, bound)` 内的下标；`bound == 0` 时返回 0。
    fn below(&mut self, bound: usize) -> usize {
        if bound == 0 {
            0
        } else {
            (self.next_u64() % bound as u64) as usize
        }
    }
}

// ---------------------------------------------------------------------------
// 游戏逻辑
// ---------------------------------------------------------------------------

/// 移动方向。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Direction {
    Up,
    Down,
    Left,
    Right,
}

/// 一次移动前可回滚的状态。
#[derive(Clone, Copy)]
struct Snapshot {
    cells: [u32; CELLS],
    score: u32,
    steps: u32,
    celebrated: bool,
}

/// 单局 2048 的完整状态。
#[derive(Clone)]
struct Game {
    cells: [u32; CELLS],
    score: u32,
    steps: u32,
    /// 上一步合并得到的分数。
    last_gain: u32,
    /// 本局是否已经结束（无路可走）。
    over: bool,
    /// 是否已经播报过「合出 2048」。
    celebrated: bool,
    /// 本局是否已经计入排行榜，避免同一局被重复累加。
    recorded: bool,
    history: Vec<Snapshot>,
}

impl Game {
    /// 新开一局：随机放置两个初始方块。
    ///
    /// 随机数生成会调用宿主接口，因此 `rng` 由调用方在**不持有任何锁**时创建。
    fn new(rng: &mut Rng) -> Self {
        let mut game = Self {
            cells: [0; CELLS],
            score: 0,
            steps: 0,
            last_gain: 0,
            over: false,
            celebrated: false,
            recorded: false,
            history: Vec::new(),
        };
        game.spawn_tile(rng);
        game.spawn_tile(rng);
        game
    }

    fn max_tile(&self) -> u32 {
        self.cells.iter().copied().max().unwrap_or(0)
    }

    fn empty_indices(&self) -> Vec<usize> {
        (0..CELLS).filter(|i| self.cells[*i] == 0).collect()
    }

    /// 在随机空格里生成一个新方块（90% 为 2，10% 为 4）。棋盘已满时返回 false。
    fn spawn_tile(&mut self, rng: &mut Rng) -> bool {
        let empties = self.empty_indices();
        if empties.is_empty() {
            return false;
        }
        let slot = empties[rng.below(empties.len())];
        self.cells[slot] = if rng.below(10) == 0 { 4 } else { 2 };
        true
    }

    /// 棋盘上是否还存在可行的操作。
    fn can_move(&self) -> bool {
        for r in 0..SIZE {
            for c in 0..SIZE {
                let value = self.cells[r * SIZE + c];
                if value == 0 {
                    return true;
                }
                if c + 1 < SIZE && value == self.cells[r * SIZE + c + 1] {
                    return true;
                }
                if r + 1 < SIZE && value == self.cells[(r + 1) * SIZE + c] {
                    return true;
                }
            }
        }
        false
    }

    /// 朝指定方向移动一步。棋盘没有变化时返回 false。
    fn shift(&mut self, dir: Direction, rng: &mut Rng) -> bool {
        let before = self.cells;
        let undo = Snapshot {
            cells: self.cells,
            score: self.score,
            steps: self.steps,
            celebrated: self.celebrated,
        };

        let mut gained = 0;
        for line in 0..SIZE {
            let source = read_line(dir, line, &self.cells);
            let (merged, line_gain) = slide(source);
            gained += line_gain;
            write_line(dir, line, &merged, &mut self.cells);
        }

        if self.cells == before {
            return false;
        }

        self.history.push(undo);
        if self.history.len() > MAX_HISTORY {
            self.history.remove(0);
        }
        self.score += gained;
        self.last_gain = gained;
        self.steps += 1;
        self.spawn_tile(rng);
        if !self.can_move() {
            self.over = true;
        }
        true
    }

    /// 回滚到上一步；没有历史时返回 false。
    fn undo(&mut self) -> bool {
        match self.history.pop() {
            Some(previous) => {
                self.cells = previous.cells;
                self.score = previous.score;
                self.steps = previous.steps;
                self.celebrated = previous.celebrated;
                self.last_gain = 0;
                self.over = !self.can_move();
                true
            }
            None => false,
        }
    }
}

/// 取出某一「行/列」，索引 0 表示移动的前沿。
fn read_line(dir: Direction, index: usize, cells: &[u32; CELLS]) -> [u32; SIZE] {
    match dir {
        Direction::Left => [
            cells[index * SIZE],
            cells[index * SIZE + 1],
            cells[index * SIZE + 2],
            cells[index * SIZE + 3],
        ],
        Direction::Right => [
            cells[index * SIZE + 3],
            cells[index * SIZE + 2],
            cells[index * SIZE + 1],
            cells[index * SIZE],
        ],
        Direction::Up => [
            cells[index],
            cells[SIZE + index],
            cells[2 * SIZE + index],
            cells[3 * SIZE + index],
        ],
        Direction::Down => [
            cells[3 * SIZE + index],
            cells[2 * SIZE + index],
            cells[SIZE + index],
            cells[index],
        ],
    }
}

/// 把处理好的「行/列」写回棋盘（索引 0 落在移动的前沿）。
fn write_line(dir: Direction, index: usize, line: &[u32; SIZE], cells: &mut [u32; CELLS]) {
    for (k, value) in line.iter().enumerate() {
        let pos = match dir {
            Direction::Left => index * SIZE + k,
            Direction::Right => index * SIZE + (SIZE - 1 - k),
            Direction::Up => k * SIZE + index,
            Direction::Down => (SIZE - 1 - k) * SIZE + index,
        };
        cells[pos] = *value;
    }
}

/// 单方向的压缩合并：相同数字相撞合并，且每个方块在一次移动中最多合并一次。
fn slide(line: [u32; SIZE]) -> ([u32; SIZE], u32) {
    let mut merged = [0u32; SIZE];
    let mut gained = 0u32;
    let compact: Vec<u32> = line.iter().copied().filter(|v| *v != 0).collect();

    let mut read = 0;
    let mut write = 0;
    while read < compact.len() {
        if read + 1 < compact.len() && compact[read] == compact[read + 1] {
            let sum = compact[read] * 2;
            gained += sum;
            merged[write] = sum;
            read += 2;
        } else {
            merged[write] = compact[read];
            read += 1;
        }
        write += 1;
    }

    (merged, gained)
}

// ---------------------------------------------------------------------------
// 多语言
// ---------------------------------------------------------------------------

/// 插件支持的语言。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Lang {
    /// 简体中文。
    ZhCn,
    /// 英文，同时是所有未收录语言的回落目标。
    EnUs,
}

impl Lang {
    /// 依据客户端语言串选择语言。
    ///
    /// 只做前缀判断：`zh_cn`、`zh_tw`、`zh_hk` 都归入简体中文——对繁体玩家来说，
    /// 简中远比英文易读；其余语言一律回落英文。
    fn from_locale(locale: &str) -> Self {
        if locale.to_ascii_lowercase().starts_with("zh") {
            Lang::ZhCn
        } else {
            Lang::EnUs
        }
    }

    fn strings(self) -> &'static dyn Strings {
        match self {
            Lang::ZhCn => &ZH_CN,
            Lang::EnUs => &EN_US,
        }
    }
}

/// 面向玩家的全部文案。
///
/// 做成 trait 而不是静态字符串表，有三个理由：
///
/// 1. `format!` 要求格式串是**字面量**，静态表里的模板是运行期字符串，用不了。
///    写成方法后模板仍是字面量，占位符个数或类型写错会直接编译失败，
///    而不是运行期拼出半截句子。
/// 2. 每种语言集中在一个 impl 里，便于整体校对译文。
/// 3. 新增语言只需补一个 impl；漏实现任何一条都是编译错误。
///
/// 注意：命令字面量不参与本地化，只有面向玩家的散文是。
trait Strings {
    // --- 通用 ---
    /// 可点击组件的悬停提示。
    fn button_hint(&self, command: &str) -> String;

    // --- 棋盘状态行 ---
    fn status_score(&self, score: u32) -> String;
    fn status_steps(&self, steps: u32) -> String;
    fn status_max(&self, tile: u32) -> String;
    fn status_rank(&self, rank: usize) -> String;
    fn status_over(&self) -> &'static str;
    fn status_won(&self) -> &'static str;

    // --- 操作栏 ---
    fn ctl_undo(&self) -> &'static str;
    fn ctl_restart(&self) -> &'static str;
    fn ctl_stop(&self) -> &'static str;
    fn ctl_best(&self) -> &'static str;
    fn ctl_me(&self) -> &'static str;

    // --- 排行榜行 ---
    fn rank_score(&self, score: u32) -> String;
    fn rank_max(&self, tile: u32) -> String;
    fn rank_games(&self, games: u32) -> String;

    // --- 没有对局 ---
    fn no_game(&self) -> &'static str;
    fn no_game_click(&self) -> &'static str;
    fn start_button(&self) -> &'static str;

    // --- 帮助 ---
    fn help_title(&self) -> &'static str;
    fn help_descriptions(&self) -> [&'static str; HELP_COUNT];
    fn help_rules(&self) -> &'static str;
    fn help_keyboard(&self) -> &'static str;
    fn help_leaderboard(&self) -> &'static str;

    // --- 排行榜 ---
    fn rank_title(&self) -> &'static str;
    fn rank_empty(&self) -> &'static str;
    fn rank_count(&self, players: usize) -> String;
    fn rank_ellipsis(&self) -> &'static str;
    fn rank_not_listed(&self) -> &'static str;
    fn rank_more(&self) -> &'static str;
    fn rank_me_button(&self) -> &'static str;

    // --- 个人战绩 ---
    fn me_title(&self) -> &'static str;
    fn me_empty(&self) -> &'static str;
    fn me_rank_label(&self) -> &'static str;
    fn me_rank_value(&self, rank: usize, total: usize) -> String;
    fn me_best_score_label(&self) -> &'static str;
    fn me_best_tile_label(&self) -> &'static str;
    fn me_games_label(&self) -> &'static str;
    fn me_average_label(&self) -> &'static str;
    fn me_steps_label(&self) -> &'static str;
    fn me_more(&self) -> &'static str;
    fn me_best_button(&self) -> &'static str;

    // --- 交互反馈 ---
    fn player_only(&self) -> &'static str;
    fn start_abandoned(&self, score: u32) -> String;
    fn start_abandoned_rank(&self, rank: usize, total: usize) -> String;
    fn start_fresh(&self) -> &'static str;
    fn undo_done(&self) -> &'static str;
    fn undo_empty(&self) -> &'static str;
    fn stop_summary(&self, score: u32) -> String;
    fn stop_recorded(&self, rank: usize, total: usize) -> String;
    fn move_over(&self) -> &'static str;
    fn move_blocked(&self) -> &'static str;
    fn move_dead(&self, score: u32) -> String;
    fn move_new_best(&self) -> &'static str;
    fn move_dead_rank(&self, rank: usize, total: usize) -> String;
    fn move_again(&self) -> &'static str;
    fn move_won(&self, gained: u32) -> String;
    fn move_gained(&self, gained: u32) -> String;
    fn move_done(&self) -> &'static str;
}

struct ZhCn;
struct EnUs;

static ZH_CN: ZhCn = ZhCn;
static EN_US: EnUs = EnUs;

impl Strings for ZhCn {
    fn button_hint(&self, command: &str) -> String {
        format!("点击执行 /{command}")
    }

    fn status_score(&self, score: u32) -> String {
        format!("得分 {score}")
    }
    fn status_steps(&self, steps: u32) -> String {
        format!("  步数 {steps}")
    }
    fn status_max(&self, tile: u32) -> String {
        format!("  最大 {tile}")
    }
    fn status_rank(&self, rank: usize) -> String {
        format!("  排名 #{rank}")
    }
    fn status_over(&self) -> &'static str {
        "  已结束"
    }
    fn status_won(&self) -> &'static str {
        "  已达成 2048"
    }

    fn ctl_undo(&self) -> &'static str {
        "撤销"
    }
    fn ctl_restart(&self) -> &'static str {
        "重开"
    }
    fn ctl_stop(&self) -> &'static str {
        "结束"
    }
    fn ctl_best(&self) -> &'static str {
        "排行"
    }
    fn ctl_me(&self) -> &'static str {
        "战绩"
    }

    fn rank_score(&self, score: u32) -> String {
        format!("  {score} 分")
    }
    fn rank_max(&self, tile: u32) -> String {
        format!("  最大 {tile}")
    }
    fn rank_games(&self, games: u32) -> String {
        format!("  {games} 局")
    }

    fn no_game(&self) -> &'static str {
        "你当前没有进行中的 2048 对局。"
    }
    fn no_game_click(&self) -> &'static str {
        "点击或输入 "
    }
    fn start_button(&self) -> &'static str {
        "开始新的一局"
    }

    fn help_title(&self) -> &'static str {
        "===== 2048 小游戏 ====="
    }
    fn help_descriptions(&self) -> [&'static str; HELP_COUNT] {
        [
            "开始 / 重开一局（别名 new、restart）",
            "向上移动（别名 w）",
            "向下移动（别名 s）",
            "向左移动（别名 a）",
            "向右移动（别名 d）",
            "撤销上一步（别名 back、z）",
            "重新显示棋盘（别名 board、b）",
            "全服排行榜前 10 名（别名 top、rank、leaderboard）",
            "我的战绩与排名（别名 stats、profile）",
            "结束当前对局（别名 quit、end、exit）",
            "显示这份说明（别名 h）",
        ]
    }
    fn help_rules(&self) -> &'static str {
        "玩法：相同数字相撞即合并，凑出 2048 即通关。"
    }
    fn help_keyboard(&self) -> &'static str {
        "键盘玩家也可以直接用 WASD 快速操作。"
    }
    fn help_leaderboard(&self) -> &'static str {
        "每局结束后成绩会自动计入排行榜并存档，重开服务器也不会丢。"
    }

    fn rank_title(&self) -> &'static str {
        "===== 2048 全服排行榜 ====="
    }
    fn rank_empty(&self) -> &'static str {
        "还没有任何记录，打出第一局就能登上榜首！"
    }
    fn rank_count(&self, players: usize) -> String {
        format!("共 {players} 位玩家上榜 · 按最高分排序")
    }
    fn rank_ellipsis(&self) -> &'static str {
        "  ……"
    }
    fn rank_not_listed(&self) -> &'static str {
        "你还没有上榜，完成一局即可参与排名。"
    }
    fn rank_more(&self) -> &'static str {
        "查看个人详情："
    }
    fn rank_me_button(&self) -> &'static str {
        "我的战绩"
    }

    fn me_title(&self) -> &'static str {
        "===== 我的 2048 战绩 ====="
    }
    fn me_empty(&self) -> &'static str {
        "你还没有完成过任何一局 2048。"
    }
    fn me_rank_label(&self) -> &'static str {
        "全服排名"
    }
    fn me_rank_value(&self, rank: usize, total: usize) -> String {
        format!("#{rank} / {total} 人")
    }
    fn me_best_score_label(&self) -> &'static str {
        "历史最高分"
    }
    fn me_best_tile_label(&self) -> &'static str {
        "最大方块"
    }
    fn me_games_label(&self) -> &'static str {
        "完成局数"
    }
    fn me_average_label(&self) -> &'static str {
        "平均得分"
    }
    fn me_steps_label(&self) -> &'static str {
        "累计步数"
    }
    fn me_more(&self) -> &'static str {
        "查看全服排名："
    }
    fn me_best_button(&self) -> &'static str {
        "排行榜"
    }

    fn player_only(&self) -> &'static str {
        "2048 是玩家专属的小游戏，请在游戏内由玩家执行该命令。"
    }
    fn start_abandoned(&self, score: u32) -> String {
        format!("新的一局开始！上一局中途放弃，得分 {score} 已计入排行榜。")
    }
    fn start_abandoned_rank(&self, rank: usize, total: usize) -> String {
        format!("上一局排名 #{rank} / {total} 人。")
    }
    fn start_fresh(&self) -> &'static str {
        "新的一局开始！方向键 / WASD / 点击下方按钮都可以移动。"
    }
    fn undo_done(&self) -> &'static str {
        "已撤销一步。"
    }
    fn undo_empty(&self) -> &'static str {
        "没有可以撤销的步骤了。"
    }
    fn stop_summary(&self, score: u32) -> String {
        format!("本局结束，最终得分 {score}。")
    }
    fn stop_recorded(&self, rank: usize, total: usize) -> String {
        format!("成绩已计入排行榜：当前排名 #{rank} / {total} 人。")
    }
    fn move_over(&self) -> &'static str {
        "本局已经结束啦，用 /2048 start 或点击【重开】再来一局。"
    }
    fn move_blocked(&self) -> &'static str {
        "这个方向没有方块可以移动，换个方向试试。"
    }
    fn move_dead(&self, score: u32) -> String {
        format!("无处可走了，本局结束！得分 {score}。")
    }
    fn move_new_best(&self) -> &'static str {
        "这是你的新纪录，已写入排行榜！"
    }
    fn move_dead_rank(&self, rank: usize, total: usize) -> String {
        format!("当前排名 #{rank} / {total} 人。用 /2048 start 再来一局。")
    }
    fn move_again(&self) -> &'static str {
        "用 /2048 start 再来一局。"
    }
    fn move_won(&self, gained: u32) -> String {
        format!("恭喜！你合出了 2048，本步 +{gained} 分。继续挑战更大数字吧！")
    }
    fn move_gained(&self, gained: u32) -> String {
        format!("合并得分 +{gained}。")
    }
    fn move_done(&self) -> &'static str {
        "移动完成。"
    }
}

impl Strings for EnUs {
    fn button_hint(&self, command: &str) -> String {
        format!("Click to run /{command}")
    }

    fn status_score(&self, score: u32) -> String {
        format!("Score {score}")
    }
    fn status_steps(&self, steps: u32) -> String {
        format!("  Steps {steps}")
    }
    fn status_max(&self, tile: u32) -> String {
        format!("  Max {tile}")
    }
    fn status_rank(&self, rank: usize) -> String {
        format!("  Rank #{rank}")
    }
    fn status_over(&self) -> &'static str {
        "  Game over"
    }
    fn status_won(&self) -> &'static str {
        "  Reached 2048"
    }

    fn ctl_undo(&self) -> &'static str {
        "Undo"
    }
    fn ctl_restart(&self) -> &'static str {
        "Restart"
    }
    fn ctl_stop(&self) -> &'static str {
        "End"
    }
    fn ctl_best(&self) -> &'static str {
        "Ranking"
    }
    fn ctl_me(&self) -> &'static str {
        "Stats"
    }

    fn rank_score(&self, score: u32) -> String {
        format!("  {score} pts")
    }
    fn rank_max(&self, tile: u32) -> String {
        format!("  Max {tile}")
    }
    fn rank_games(&self, games: u32) -> String {
        format!("  {games} games")
    }

    fn no_game(&self) -> &'static str {
        "You have no 2048 game in progress."
    }
    fn no_game_click(&self) -> &'static str {
        "Click or type "
    }
    fn start_button(&self) -> &'static str {
        "Start a game"
    }

    fn help_title(&self) -> &'static str {
        "===== 2048 ====="
    }
    fn help_descriptions(&self) -> [&'static str; HELP_COUNT] {
        [
            "Start or restart a game (aliases new, restart)",
            "Move up (alias w)",
            "Move down (alias s)",
            "Move left (alias a)",
            "Move right (alias d)",
            "Undo the previous move (aliases back, z)",
            "Re-display the board (aliases board, b)",
            "Server-wide top 10 (aliases top, rank, leaderboard)",
            "Your own stats and rank (aliases stats, profile)",
            "End the current game (aliases quit, end, exit)",
            "Show this reference (alias h)",
        ]
    }
    fn help_rules(&self) -> &'static str {
        "Rules: equal tiles merge when they collide; reach 2048 to win."
    }
    fn help_keyboard(&self) -> &'static str {
        "Keyboard players can also just use WASD."
    }
    fn help_leaderboard(&self) -> &'static str {
        "Every finished game is recorded to the leaderboard and saved to disk, so a server restart will not lose it."
    }

    fn rank_title(&self) -> &'static str {
        "===== 2048 leaderboard ====="
    }
    fn rank_empty(&self) -> &'static str {
        "No records yet - play one game to top the board!"
    }
    fn rank_count(&self, players: usize) -> String {
        format!("{players} players on the board · sorted by best score")
    }
    fn rank_ellipsis(&self) -> &'static str {
        "  ..."
    }
    fn rank_not_listed(&self) -> &'static str {
        "You are not on the board yet - finish a game to join."
    }
    fn rank_more(&self) -> &'static str {
        "Your own details:"
    }
    fn rank_me_button(&self) -> &'static str {
        "My stats"
    }

    fn me_title(&self) -> &'static str {
        "===== My 2048 stats ====="
    }
    fn me_empty(&self) -> &'static str {
        "You have not finished a 2048 game yet."
    }
    fn me_rank_label(&self) -> &'static str {
        "Server rank"
    }
    fn me_rank_value(&self, rank: usize, total: usize) -> String {
        format!("#{rank} / {total} players")
    }
    fn me_best_score_label(&self) -> &'static str {
        "Best score"
    }
    fn me_best_tile_label(&self) -> &'static str {
        "Largest tile"
    }
    fn me_games_label(&self) -> &'static str {
        "Games"
    }
    fn me_average_label(&self) -> &'static str {
        "Average score"
    }
    fn me_steps_label(&self) -> &'static str {
        "Total steps"
    }
    fn me_more(&self) -> &'static str {
        "Server ranking:"
    }
    fn me_best_button(&self) -> &'static str {
        "Leaderboard"
    }

    fn player_only(&self) -> &'static str {
        "2048 is a player-only mini-game; run this command in game as a player."
    }
    fn start_abandoned(&self, score: u32) -> String {
        format!("New game! The previous one was abandoned and its score of {score} was recorded.")
    }
    fn start_abandoned_rank(&self, rank: usize, total: usize) -> String {
        format!("Previous game rank #{rank} / {total} players.")
    }
    fn start_fresh(&self) -> &'static str {
        "New game! Use the arrow keys / WASD, or click a button below, to move."
    }
    fn undo_done(&self) -> &'static str {
        "Undid one move."
    }
    fn undo_empty(&self) -> &'static str {
        "Nothing left to undo."
    }
    fn stop_summary(&self, score: u32) -> String {
        format!("Game ended. Final score {score}.")
    }
    fn stop_recorded(&self, rank: usize, total: usize) -> String {
        format!("Score recorded on the leaderboard: rank #{rank} / {total} players.")
    }
    fn move_over(&self) -> &'static str {
        "This game is already over - run /2048 start or click [Restart] to play again."
    }
    fn move_blocked(&self) -> &'static str {
        "No tiles can move in that direction; try another."
    }
    fn move_dead(&self, score: u32) -> String {
        format!("No moves left - game over! Score {score}.")
    }
    fn move_new_best(&self) -> &'static str {
        "That is your new record - written to the leaderboard!"
    }
    fn move_dead_rank(&self, rank: usize, total: usize) -> String {
        format!("Rank #{rank} / {total} players. Run /2048 start to play again.")
    }
    fn move_again(&self) -> &'static str {
        "Run /2048 start to play again."
    }
    fn move_won(&self, gained: u32) -> String {
        format!("Congratulations! You made 2048, +{gained} points this move. Keep going for a bigger tile!")
    }
    fn move_gained(&self, gained: u32) -> String {
        format!("Merged for +{gained} points.")
    }
    fn move_done(&self) -> &'static str {
        "Move complete."
    }
}

// ---------------------------------------------------------------------------
// 文本渲染
// ---------------------------------------------------------------------------

fn colored(text: &str, color: NamedColor) -> TextComponent {
    TextComponent::text(text).color_named(color)
}

fn good(text: &str) -> TextComponent {
    colored(text, NamedColor::Green)
}

fn bad(text: &str) -> TextComponent {
    colored(text, NamedColor::Red)
}

fn tip(text: &str) -> TextComponent {
    colored(text, NamedColor::Yellow)
}

fn dim(text: &str) -> TextComponent {
    colored(text, NamedColor::Gray)
}

/// 一个可点击的按钮：点击后自动执行对应命令。
fn button(lang: Lang, command: &str, label: &str, color: NamedColor) -> TextComponent {
    let hint = colored(&lang.strings().button_hint(command), NamedColor::Gray);
    TextComponent::text(&format!("[{label}] "))
        .color_named(color)
        .bold(true)
        .click_run_command(command)
        .hover_show_text(hint)
}

/// 名次对应的颜色：前三名有专门的「奖牌色」。
fn rank_color(rank: usize) -> NamedColor {
    match rank {
        1 => NamedColor::Gold,
        2 => NamedColor::Gray,
        3 => NamedColor::DarkRed,
        _ => NamedColor::DarkGray,
    }
}

/// 排行榜中的一行：`名次. 玩家  分数  最大方块  局数`。
fn rank_line(lang: Lang, rank: usize, profile: &Profile, highlight: bool) -> TextComponent {
    let s = lang.strings();
    let mut line = TextComponent::text("\n");
    line = line.add_child(colored(&format!("{rank:>2}. "), rank_color(rank)));

    let mut name = colored(&profile.name, if highlight { NamedColor::Green } else { NamedColor::White });
    if highlight {
        name = name.bold(true);
    }
    line = line.add_child(name);

    line = line.add_child(colored(&s.rank_score(profile.best_score), NamedColor::Yellow));
    line = line.add_child(colored(&s.rank_max(profile.best_tile), NamedColor::LightPurple));
    line = line.add_child(colored(&s.rank_games(profile.games), NamedColor::Gray));
    line
}

/// 「标签 值」形式的一行战绩。
fn stat_line(label: &str, value: &str, color: NamedColor) -> TextComponent {
    let mut line = TextComponent::text("\n");
    line = line.add_child(colored(&format!("  {label} "), NamedColor::Gray));
    line = line.add_child(colored(value, color));
    line
}

/// 与数字对应的颜色，让棋盘一眼可读。
fn tile_color(value: u32) -> NamedColor {
    match value {
        0 => NamedColor::DarkGray,
        2 => NamedColor::Gray,
        4 => NamedColor::White,
        8 => NamedColor::Gold,
        16 => NamedColor::Yellow,
        32 => NamedColor::Red,
        64 => NamedColor::DarkRed,
        128 => NamedColor::Aqua,
        256 => NamedColor::DarkAqua,
        512 => NamedColor::Blue,
        1024 => NamedColor::LightPurple,
        2048 => NamedColor::Green,
        4096 => NamedColor::DarkGreen,
        8192 => NamedColor::DarkPurple,
        _ => NamedColor::White,
    }
}

/// 单个棋格（固定 6 字符宽，保证聊天栏里上下对齐）。
fn tile(value: u32) -> TextComponent {
    let label = if value == 0 {
        "-".to_string()
    } else {
        value.to_string()
    };
    let cell = colored(&format!(" {:^6}", label), tile_color(value));
    if value >= 128 {
        cell.bold(true)
    } else {
        cell
    }
}

/// 顶部状态行，顺带带上玩家当前的全服名次。
fn status_line(lang: Lang, game: &Game, rank: Option<usize>) -> TextComponent {
    let s = lang.strings();
    let mut line = TextComponent::text("");
    line = line.add_child(
        TextComponent::text("2048 ")
            .color_named(NamedColor::Gold)
            .bold(true),
    );
    line = line.add_child(colored(&s.status_score(game.score), NamedColor::Yellow));
    line = line.add_child(colored(&s.status_steps(game.steps), NamedColor::Aqua));
    line = line.add_child(colored(&s.status_max(game.max_tile()), NamedColor::LightPurple));
    if let Some(rank) = rank {
        line = line.add_child(colored(&s.status_rank(rank), NamedColor::Blue));
    }

    if game.over {
        line = line.add_child(
            TextComponent::text(s.status_over())
                .color_named(NamedColor::Red)
                .bold(true),
        );
    } else if game.max_tile() >= TARGET {
        line = line.add_child(
            TextComponent::text(s.status_won())
                .color_named(NamedColor::Green)
                .bold(true),
        );
    }
    line
}

/// 底部可点击的操作栏。
fn control_row(lang: Lang) -> TextComponent {
    let s = lang.strings();
    let mut row = TextComponent::text("");
    row = row.add_child(button(lang, "2048 up", "↑W", NamedColor::Green));
    row = row.add_child(button(lang, "2048 left", "←A", NamedColor::Green));
    row = row.add_child(button(lang, "2048 down", "↓S", NamedColor::Green));
    row = row.add_child(button(lang, "2048 right", "→D", NamedColor::Green));
    row = row.add_child(TextComponent::text("  "));
    row = row.add_child(button(lang, "2048 undo", s.ctl_undo(), NamedColor::Yellow));
    row = row.add_child(button(lang, "2048 start", s.ctl_restart(), NamedColor::Gold));
    row = row.add_child(button(lang, "2048 stop", s.ctl_stop(), NamedColor::Red));
    row = row.add_child(button(lang, "2048 best", s.ctl_best(), NamedColor::Aqua));
    row = row.add_child(button(lang, "2048 me", s.ctl_me(), NamedColor::LightPurple));
    row
}

/// 完整的棋盘消息：状态行 + 4 行棋盘 + 操作栏 + 可选提示。
fn board_message(
    lang: Lang,
    game: &Game,
    rank: Option<usize>,
    note: Option<TextComponent>,
) -> TextComponent {
    let mut root = TextComponent::text("");
    root = root.add_child(status_line(lang, game, rank));

    for r in 0..SIZE {
        root = root.add_child(TextComponent::text("\n"));
        let mut row = TextComponent::text(" ");
        for c in 0..SIZE {
            row = row.add_child(tile(game.cells[r * SIZE + c]));
        }
        root = root.add_child(row);
    }

    root = root.add_child(TextComponent::text("\n"));
    root = root.add_child(control_row(lang));

    if let Some(note) = note {
        root = root.add_child(TextComponent::text("\n"));
        root = root.add_child(note);
    }
    root
}

/// 带名次信息的棋盘（渲染时实时查询榜单）。
fn board_with(
    lang: Lang,
    game: &Game,
    owner: &str,
    note: Option<TextComponent>,
) -> TextComponent {
    board_message(lang, game, rank_of(owner), note)
}

fn no_game_message(lang: Lang) -> TextComponent {
    let s = lang.strings();
    let mut root = tip(s.no_game());
    root = root.add_child(TextComponent::text("\n"));
    root = root.add_child(dim(s.no_game_click()));
    root = root.add_child(button(lang, "2048 start", s.start_button(), NamedColor::Green));
    root
}

fn help_message(lang: Lang) -> TextComponent {
    let s = lang.strings();
    let mut root = TextComponent::text("");
    root = root.add_child(
        TextComponent::text(s.help_title())
            .color_named(NamedColor::Gold)
            .bold(true),
    );

    let descriptions = s.help_descriptions();
    for (command, description) in HELP_COMMANDS.iter().zip(descriptions.iter()) {
        let mut line = TextComponent::text("\n");
        line = line.add_child(dim("  /"));
        line = line.add_child(button(lang, command, command, NamedColor::Aqua));
        line = line.add_child(dim(&format!("- {description}")));
        root = root.add_child(line);
    }

    root = root.add_child(TextComponent::text("\n"));
    root = root.add_child(tip(s.help_rules()));
    root = root.add_child(TextComponent::text("\n"));
    root = root.add_child(dim(s.help_keyboard()));
    root = root.add_child(TextComponent::text("\n"));
    root = root.add_child(dim(s.help_leaderboard()));
    root
}

/// 全服排行榜：前 `TOP_N` 名 + 请求者自己的名次。
fn ranking_message(lang: Lang, owner: &str) -> TextComponent {
    let s = lang.strings();
    let sorted = sorted_profiles();

    let mut root = TextComponent::text("");
    root = root.add_child(
        TextComponent::text(s.rank_title())
            .color_named(NamedColor::Gold)
            .bold(true),
    );

    if sorted.is_empty() {
        root = root.add_child(TextComponent::text("\n"));
        root = root.add_child(dim(s.rank_empty()));
        root = root.add_child(TextComponent::text("\n"));
        root = root.add_child(button(lang, "2048 start", s.start_button(), NamedColor::Green));
        return root;
    }

    root = root.add_child(TextComponent::text("\n"));
    root = root.add_child(dim(&s.rank_count(sorted.len())));

    for (index, profile) in sorted.iter().take(TOP_N).enumerate() {
        root = root.add_child(rank_line(lang, index + 1, profile, profile.name == owner));
    }

    match sorted.iter().position(|p| p.name == owner) {
        Some(index) if index + 1 > TOP_N => {
            root = root.add_child(TextComponent::text("\n"));
            root = root.add_child(dim(s.rank_ellipsis()));
            root = root.add_child(rank_line(lang, index + 1, &sorted[index], true));
        }
        Some(_) => {}
        None => {
            root = root.add_child(TextComponent::text("\n"));
            root = root.add_child(dim(s.rank_not_listed()));
        }
    }

    root = root.add_child(TextComponent::text("\n"));
    root = root.add_child(dim(s.rank_more()));
    root = root.add_child(button(lang, "2048 me", s.rank_me_button(), NamedColor::LightPurple));
    root
}

/// 个人战绩详情。
fn me_message(lang: Lang, owner: &str) -> TextComponent {
    let s = lang.strings();
    let sorted = sorted_profiles();

    let mut root = TextComponent::text("");
    root = root.add_child(
        TextComponent::text(s.me_title())
            .color_named(NamedColor::Gold)
            .bold(true),
    );

    let Some(profile) = sorted.iter().find(|p| p.name == owner) else {
        root = root.add_child(TextComponent::text("\n"));
        root = root.add_child(dim(s.me_empty()));
        root = root.add_child(TextComponent::text("\n"));
        root = root.add_child(button(lang, "2048 start", s.start_button(), NamedColor::Green));
        return root;
    };

    let rank = sorted
        .iter()
        .position(|p| p.name == owner)
        .map_or(1, |i| i + 1);
    let average = if profile.games > 0 {
        profile.score_sum / profile.games as u64
    } else {
        0
    };

    root = root.add_child(stat_line(
        s.me_rank_label(),
        &s.me_rank_value(rank, sorted.len()),
        NamedColor::Aqua,
    ));
    root = root.add_child(stat_line(
        s.me_best_score_label(),
        &profile.best_score.to_string(),
        NamedColor::Yellow,
    ));
    root = root.add_child(stat_line(
        s.me_best_tile_label(),
        &profile.best_tile.to_string(),
        NamedColor::LightPurple,
    ));
    root = root.add_child(stat_line(
        s.me_games_label(),
        &profile.games.to_string(),
        NamedColor::Gray,
    ));
    root = root.add_child(stat_line(
        s.me_average_label(),
        &average.to_string(),
        NamedColor::Green,
    ));
    root = root.add_child(stat_line(
        s.me_steps_label(),
        &profile.steps.to_string(),
        NamedColor::Gray,
    ));

    root = root.add_child(TextComponent::text("\n"));
    root = root.add_child(dim(s.me_more()));
    root = root.add_child(button(lang, "2048 best", s.me_best_button(), NamedColor::Aqua));
    root
}

// ---------------------------------------------------------------------------
// 命令分发
// ---------------------------------------------------------------------------

/// 玩家可以触发的动作。
#[derive(Clone, Copy)]
enum Action {
    Start,
    Move(Direction),
    Show,
    Undo,
    Stop,
    Best,
    Me,
    Help,
}

/// 一次移动之后要展示的结果。
enum MoveNote {
    /// 对局已经结束。
    Over,
    /// 该方向无法产生任何位移。
    Blocked,
    /// 移动成功。
    Moved {
        gained: u32,
        won_now: bool,
        over_now: bool,
    },
}

/// 所有子命令共用的执行器。
#[derive(Clone, Copy)]
struct Executor(Action);

impl CommandHandler for Executor {
    fn handle(
        &self,
        sender: CommandSender,
        _server: Server,
        _args: ConsumedArgs,
    ) -> Result<i32, CommandError> {
        play(&sender, self.0);
        Ok(1)
    }
}

/// 发送者使用的语言。
///
/// 从玩家客户端上报的 locale 取；控制台、命令方块等没有语言的来源一律回落英文
/// （服务端日志与控制台的惯用语言）。
fn sender_lang(sender: &CommandSender) -> Lang {
    sender
        .as_player()
        .map_or(Lang::EnUs, |player| Lang::from_locale(&player.get_locale()))
}

/// 统一的入口：先取出需要的数据（避免在持有锁时调用宿主 API），再渲染并回复玩家。
fn play(sender: &CommandSender, action: Action) {
    // 语言每执行一条命令解析一次，随后作为 `Lang` 逐层传下去，
    // 因此整条命令最多只发生一次 locale 查询。
    let lang = sender_lang(sender);
    let s = lang.strings();

    if !sender.is_player() {
        sender.send_message(colored(s.player_only(), NamedColor::Red));
        return;
    }

    let name = sender.get_name();
    let message = match action {
        Action::Help => help_message(lang),
        Action::Best => ranking_message(lang, &name),
        Action::Me => me_message(lang, &name),
        Action::Show => match snapshot_of(&name) {
            Some(game) => board_with(lang, &game, &name, None),
            None => no_game_message(lang),
        },
        Action::Start => {
            // 随机数种子来自宿主接口，必须在持锁之前取好。
            let mut rng = Rng::new();
            let (game, abandoned) = {
                let mut list = games();
                match index_of(&list, &name) {
                    Some(i) => {
                        let old = std::mem::replace(&mut list[i].game, Game::new(&mut rng));
                        let abandoned = if old.steps > 0 && !old.recorded {
                            Some(old)
                        } else {
                            None
                        };
                        (list[i].game.clone(), abandoned)
                    }
                    None => {
                        let fresh = Game::new(&mut rng);
                        list.push(Entry {
                            owner: name.clone(),
                            game: fresh.clone(),
                        });
                        (fresh, None)
                    }
                }
            };

            let note = match abandoned {
                Some(old) => {
                    let score = old.score;
                    let result = finish_game(&name, &old);
                    let mut line = good(&s.start_abandoned(score));
                    if let Some(rank) = result.rank {
                        line = line.add_child(TextComponent::text("\n"));
                        line = line.add_child(dim(&s.start_abandoned_rank(rank, result.total)));
                    }
                    line
                }
                None => good(s.start_fresh()),
            };
            board_with(lang, &game, &name, Some(note))
        }
        Action::Undo => {
            let outcome = {
                let mut list = games();
                match index_of(&list, &name) {
                    Some(i) => {
                        let game = &mut list[i].game;
                        let ok = game.undo();
                        Some((game.clone(), ok))
                    }
                    None => None,
                }
            };
            match outcome {
                None => no_game_message(lang),
                Some((game, true)) => board_with(lang, &game, &name, Some(dim(s.undo_done()))),
                Some((game, false)) => {
                    board_with(lang, &game, &name, Some(tip(s.undo_empty())))
                }
            }
        }
        Action::Stop => {
            let removed = {
                let mut list = games();
                index_of(&list, &name).map(|i| list.remove(i).game)
            };
            match removed {
                None => no_game_message(lang),
                Some(game) => {
                    let mut line = dim(&s.stop_summary(game.score));
                    if game.steps > 0 && !game.recorded {
                        let mut recorded = game.clone();
                        recorded.recorded = true;
                        let result = finish_game(&name, &recorded);
                        if let Some(rank) = result.rank {
                            line = line.add_child(TextComponent::text("\n"));
                            line = line.add_child(dim(&s.stop_recorded(rank, result.total)));
                        }
                    }
                    board_with(lang, &game, &name, Some(line))
                }
            }
        }
        Action::Move(dir) => {
            let mut rng = Rng::new();
            let outcome: Option<(Game, MoveNote, bool)> = {
                let mut list = games();
                match index_of(&list, &name) {
                    None => None,
                    Some(i) => {
                        let game = &mut list[i].game;
                        if game.over {
                            Some((game.clone(), MoveNote::Over, false))
                        } else if !game.shift(dir, &mut rng) {
                            Some((game.clone(), MoveNote::Blocked, false))
                        } else {
                            let gained = game.last_gain;
                            let won_now = game.max_tile() >= TARGET && !game.celebrated;
                            if won_now {
                                game.celebrated = true;
                            }
                            let over_now = game.over;
                            let mut record = false;
                            if over_now && !game.recorded {
                                game.recorded = true;
                                record = true;
                            }
                            Some((
                                game.clone(),
                                MoveNote::Moved {
                                    gained,
                                    won_now,
                                    over_now,
                                },
                                record,
                            ))
                        }
                    }
                }
            };

            match outcome {
                None => no_game_message(lang),
                Some((game, note, record)) => {
                    let line = match note {
                        MoveNote::Over => bad(s.move_over()),
                        MoveNote::Blocked => tip(s.move_blocked()),
                        MoveNote::Moved {
                            gained,
                            won_now,
                            over_now,
                        } => {
                            if over_now {
                                let result = if record {
                                    finish_game(&name, &game)
                                } else {
                                    let total = profiles().len();
                                    GameResult {
                                        rank: rank_of(&name),
                                        total,
                                        new_best: false,
                                    }
                                };
                                let mut line = bad(&s.move_dead(game.score));
                                if result.new_best {
                                    line = line.add_child(TextComponent::text("\n"));
                                    line = line.add_child(good(s.move_new_best()));
                                }
                                if let Some(rank) = result.rank {
                                    line = line.add_child(TextComponent::text("\n"));
                                    line = line.add_child(dim(&s.move_dead_rank(rank, result.total)));
                                } else {
                                    line = line.add_child(TextComponent::text("\n"));
                                    line = line.add_child(dim(s.move_again()));
                                }
                                line
                            } else if won_now {
                                good(&s.move_won(gained))
                            } else if gained > 0 {
                                dim(&s.move_gained(gained))
                            } else {
                                dim(s.move_done())
                            }
                        }
                    };
                    board_with(lang, &game, &name, Some(line))
                }
            }
        }
    };

    sender.send_message(message);
}

// ---------------------------------------------------------------------------
// 插件入口
// ---------------------------------------------------------------------------

/// 插件本体。
struct TwoZeroFour;

impl Plugin for TwoZeroFour {
    fn new() -> Self {
        TwoZeroFour
    }

    fn metadata(&self) -> PluginMetadata {
        PluginMetadata {
            name: "two048".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            authors: vec!["WorkBuddy".into()],
            description: PLUGIN_DESCRIPTION.into(),
            dependencies: Vec::new(),
            // 排行榜需要读写插件私有数据目录。
            permissions: vec![FS_READ_DATA.to_string(), FS_WRITE_DATA.to_string()],
        }
    }

    fn on_load(&self, context: Context) -> pumpkin_plugin_api::Result<()> {
        context.register_permission(&Permission {
            node: PERM_NODE.to_string(),
            description: "允许玩家游玩 2048 小游戏".to_string(),
            default: PermissionDefault::Allow,
            children: Vec::new(),
        })?;

        // 载入持久化的排行榜。
        let path = format!("{}/{BOARD_FILE}", context.get_data_folder());
        {
            let mut slot = DATA_FILE.lock().unwrap_or_else(|e| e.into_inner());
            *slot = Some(path.clone());
        }
        load_profiles(&path);
        tracing::info!("two048: 排行榜已从 {path} 载入 {} 条记录", profiles().len());

        let names = [
            "2048".to_string(),
            "t2048".to_string(),
            "game2048".to_string(),
        ];
        let mut command = Command::new(&names, COMMAND_DESCRIPTION);

        // 直接执行 /2048 时展示棋盘（或引导开始）。
        command = command.execute(Executor(Action::Show));

        // 开始 / 重开
        for alias in ["start", "new", "restart", "reset"] {
            command = command.then(CommandNode::literal(alias).execute(Executor(Action::Start)));
        }
        // 四个方向（含 WASD 别名）
        for alias in ["up", "w"] {
            command = command.then(
                CommandNode::literal(alias).execute(Executor(Action::Move(Direction::Up))),
            );
        }
        for alias in ["down", "s"] {
            command = command.then(
                CommandNode::literal(alias).execute(Executor(Action::Move(Direction::Down))),
            );
        }
        for alias in ["left", "a"] {
            command = command.then(
                CommandNode::literal(alias).execute(Executor(Action::Move(Direction::Left))),
            );
        }
        for alias in ["right", "d"] {
            command = command.then(
                CommandNode::literal(alias).execute(Executor(Action::Move(Direction::Right))),
            );
        }
        // 撤销
        for alias in ["undo", "back", "z"] {
            command = command.then(CommandNode::literal(alias).execute(Executor(Action::Undo)));
        }
        // 重新显示
        for alias in ["show", "board", "b"] {
            command = command.then(CommandNode::literal(alias).execute(Executor(Action::Show)));
        }
        // 结束
        for alias in ["stop", "quit", "end", "exit"] {
            command = command.then(CommandNode::literal(alias).execute(Executor(Action::Stop)));
        }
        // 排行榜
        for alias in ["best", "top", "rank", "leaderboard"] {
            command = command.then(CommandNode::literal(alias).execute(Executor(Action::Best)));
        }
        // 个人战绩
        for alias in ["me", "stats", "profile"] {
            command = command.then(CommandNode::literal(alias).execute(Executor(Action::Me)));
        }
        // 帮助
        for alias in ["help", "h"] {
            command = command.then(CommandNode::literal(alias).execute(Executor(Action::Help)));
        }

        context.register_command(command, PERM_NODE);

        tracing::info!("two048 已加载：使用 /2048 start 开始一局 2048");
        Ok(())
    }

    fn on_unload(&self, _context: Context) -> pumpkin_plugin_api::Result<()> {
        save_profiles();
        games().clear();
        tracing::info!("two048 已卸载，排行榜已保存");
        Ok(())
    }
}

pumpkin_plugin_api::register_plugin!(TwoZeroFour);
