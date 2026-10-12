//! Rules of the 0.26g client that decide what its player is shown, beside
//! the layouts [`super::client_model`] reads: for tests of the server's
//! feedback (`social.md` §9).
//!
//! - [`denial_is_shown`]: which `84 <op>` put the game's text on screen.
//! - [`ChatSpamFilter`]: the client's own chat spam filter. It, not the
//!   server, prints the game's "The spam filter has blocked your message"
//!   line, and it does not send the line it blocks.
//! - [`connects_to_match`]: which `82` make the client connect to the
//!   match server at all.
//! - [`RankingPage`]: how the ranking page pairs the rows of a `91` with
//!   the rows it asked for.

use std::collections::{HashMap, VecDeque};

use super::channel;
use super::requests::id;

/// Whether the client shows the text of `84 <op> 00 <text>`
/// (`lobby_menu::on_operation_denied_received` 0x21D9D0, PROVEN).
///
/// The handler localizes the text (0x1FF0F0), then jumps by `op - 0x2F`
/// (byte table 0x21DED8, jump table 0x21DEB8; `cmp eax,0x1B; ja` at
/// 0x21DA0B):
/// - `2F`, `30`: `root.show_system_message(0, "debug", text)` and the Play
///   button is reset (`st_not_logged_id` signs out instead);
/// - `46`, `47`: the same window, then the inventory is drawn again;
/// - `48`, `49`: the same window and nothing else (0x21DCF1, 0x21DD6F): a
///   message box with the text and OK, whatever the client was doing;
/// - `4A`: returns at once (0x21DEA8): nothing is shown;
/// - `31`..=`45` and anything past `4A` (so `4B`..=`50`, the chat and
///   friends ops among them): only the log line `Unknown (operation
///   denied) type received %d` (0x21DDED).
pub fn denial_is_shown(op: u8) -> bool {
    matches!(
        op,
        id::PLAY | id::CLAN_PLAY | id::INVENTORY | id::SHOP | id::SKILLS | id::PROFILE
    )
}

/// The `Match id` a client holds before its first static info (the static
/// info's constructor, 0x3183D2).
pub const NO_STATIC_ID: u32 = u32::MAX;

/// Whether the client connects to the match server on `82 .. match_id ..`
/// (`network_client::connect_to_match` 0x21BEB0, PROVEN).
///
/// `held` is the `Match id` of the last static info (0xE4) the client took
/// since it was started ([`NO_STATIC_ID`] before the first): the field is
/// written by the static info reader (0x1F0E7A) and the constructor only,
/// so it outlives the match. The function's first act is to compare it
/// with the `82`'s match id and return when they are equal
/// (0x21BEBC..0x21BED5): no log line, no 0xD8, and the client, already in
/// state 2, waits on "matchmaking is over, waiting for the server". The
/// `82` handler (0x274A46) has no other condition.
pub fn connects_to_match(held: u32, match_id: u32) -> bool {
    held != match_id
}

/// The ranking page's "top 100" list as the game's script keeps it
/// (`survarium/controls/PlayerRanking.as`; PROVEN in the script, tab 0,
/// not scrolled). `R` is a row of a `91`.
///
/// The page asks for rows by index (`request_players_range` → `3B`) and
/// remembers the indexes in `_requestList`; `fillPlayers` gives each row
/// that comes to the next index of that list, whatever the row says. The
/// list is cleared nowhere: an answer that never comes shifts every later
/// page for as long as the game runs.
#[derive(Clone, Debug)]
pub struct RankingPage<R> {
    /// `_totalPlayers`: what the last `9D` handed over
    /// (`ranking_set_players_total_count`).
    total: usize,
    /// `_requestList`.
    asked: VecDeque<usize>,
    /// `_playersCache`.
    cache: HashMap<usize, R>,
    /// `_topLeague`.
    top: Vec<Option<R>>,
}

impl<R: Clone> Default for RankingPage<R> {
    fn default() -> Self {
        Self {
            total: 0,
            asked: VecDeque::new(),
            cache: HashMap::new(),
            top: Vec::new(),
        }
    }
}

impl<R: Clone> RankingPage<R> {
    /// The rows the page asks for at once at most.
    const PAGE: usize = 100;

    pub fn new() -> Self {
        Self::default()
    }

    /// `set totalPlayers`: a `9D` came. (On this tab, not scrolled, its
    /// `show_players_list` asks for nothing.)
    pub fn set_total(&mut self, total: usize) {
        self.total = total;
        self.top.resize(total.min(Self::PAGE), None);
    }

    /// `onActivate`: the page is opened. The cache and the rows shown are
    /// emptied, the list of rows asked for is not. Returns the `3B` it
    /// sends, `(start, count)`, if any.
    pub fn open(&mut self) -> Option<(u32, u8)> {
        self.cache.clear();
        self.top.fill(None);
        self.request(0, Self::PAGE)
    }

    /// `request_players(start, count)`.
    fn request(&mut self, start: usize, count: usize) -> Option<(u32, u8)> {
        let count = count
            .min(self.total.saturating_sub(start))
            .min(Self::PAGE.saturating_sub(start));
        let missing = |index: &usize| !self.cache.contains_key(index);
        let first = (start..start + count).find(missing)?;
        let last = (start..start + count).rev().find(missing)?;
        self.asked.extend(first..=last);
        Some((first as u32, (last - first + 1) as u8))
    }

    /// `fillPlayers`: the rows of a `91`, in order.
    pub fn fill(&mut self, rows: &[R]) {
        for row in rows {
            // `shift()` of an empty array is `undefined`: 0 as an `int`.
            let index = self.asked.pop_front().unwrap_or(0);
            self.cache.insert(index, row.clone());
        }
        for (index, shown) in self.top.iter_mut().enumerate() {
            if shown.is_none() {
                *shown = self.cache.get(&index).cloned();
            }
        }
    }

    /// The lines of the list, top down; `None` is an empty line.
    pub fn shown(&self) -> &[Option<R>] {
        &self.top
    }

    /// The row indexes the page still waits for.
    pub fn waits_for(&self) -> usize {
        self.asked.len()
    }
}

/// The lines within [`ChatSpamFilter::WINDOW_MS`] that mute the client.
const LINES: usize = 3;

/// The client's chat spam filter (0x2083F0, called from the chat input's
/// "send" at 0x2086E8; PROVEN). Times are the client's clock in
/// milliseconds (lobby_menu's owner +0x584C).
///
/// It runs **only for the general, match and team channels** (1, 5, 6, 7:
/// `cmp eax,1 / 5 / 6 / 7` at 0x2086C6..0x2086D8). A line on any other
/// channel (private 4, clan 3, squad 8, clan team 9) is always sent.
///
/// For a line it blocks, the client prints on its system channel (2), from
/// the sender `System`, `"%s %2.1f %s"` of `st_spam_filter_block_notification`,
/// the seconds left and `st_spam_filter_block_notification_fin`
/// (0x208737..0x2087E3), and **does not send it** (the sender 0x2089D0 is
/// only reached with no wait, 0x208731).
#[derive(Clone, Debug, Default)]
pub struct ChatSpamFilter {
    /// +0x28: no line passes before this.
    muted_until: u32,
    /// +0x2C: the last line longer than [`Self::LONG_LINE`] (the client
    /// keeps its CRC-32, 0x1C180).
    last_long: Option<String>,
    /// +0x30..+0x44: when the last lines passed (a ring of four of which
    /// three are compared).
    passed: Vec<u32>,
}

impl ChatSpamFilter {
    /// A line of more characters than this is "long" (`cmp eax,0x14; jna`).
    pub const LONG_LINE: usize = 20;
    /// The same long line is not sent again within this (0x2710).
    pub const REPEAT_MS: u32 = 10_000;
    /// Three lines within this mute the client (0x7D0).
    pub const WINDOW_MS: u32 = 2_000;
    /// The mute on the general channel, and on the others (0x208527:
    /// `10 - 5 * (channel != 1)` seconds).
    pub const MUTE_GENERAL_MS: u32 = 10_000;
    pub const MUTE_OTHER_MS: u32 = 5_000;

    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the client runs the filter for a line on `channel`.
    pub fn applies_to(channel: u8) -> bool {
        matches!(
            channel,
            channel::GENERAL | channel::MATCH | channel::TEAM_1 | channel::TEAM_2
        )
    }

    /// The player sends `text` on `channel` at `now`: `Ok` if the client
    /// sends it, or the milliseconds its spam notice names.
    pub fn send(&mut self, now: u32, text: &str, channel: u8) -> Result<(), u32> {
        if !Self::applies_to(channel) {
            return Ok(());
        }
        // 0x2083FE: still muted.
        if now < self.muted_until {
            return Err(self.muted_until - now);
        }
        // 0x20841D: the same long line again, too soon after the last line.
        if text.len() > Self::LONG_LINE {
            if self.last_long.as_deref() == Some(text)
                && let Some(&last) = self.passed.last()
                && now < last.saturating_add(Self::REPEAT_MS)
            {
                return Err(last + Self::REPEAT_MS - now);
            }
            self.last_long = Some(text.to_owned());
        } else {
            self.last_long = None;
        }
        // 0x2084DA: the line's time joins the ring; the third within the
        // window is blocked and mutes.
        self.passed.push(now);
        if self.passed.len() > LINES {
            self.passed.remove(0);
        }
        if self.passed.len() == LINES && now - self.passed[0] < Self::WINDOW_MS {
            let mute = if channel == channel::GENERAL {
                Self::MUTE_GENERAL_MS
            } else {
                Self::MUTE_OTHER_MS
            };
            self.muted_until = now.saturating_add(mute);
            return Err(mute);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A server for [`RankingPage`]: `players` ranked, answering as srp's
    /// lobby does.
    fn answer(players: u32, asked: Option<(u32, u8)>) -> Vec<u32> {
        let Some((start, count)) = asked else {
            return Vec::new();
        };
        (start + 1..=(start + u32::from(count)).min(players)).collect()
    }

    fn lines(page: &RankingPage<u32>) -> Vec<u32> {
        page.shown().iter().map(|row| row.unwrap_or(0)).collect()
    }

    #[test]
    fn the_ranking_page_is_right_while_every_request_is_answered() {
        let mut page = RankingPage::new();
        page.set_total(3);
        let asked = page.open();
        assert_eq!(asked, Some((0, 3)));
        page.fill(&answer(3, asked));
        assert_eq!(lines(&page), [1, 2, 3]);
        // A fourth player: the page asks by the count it has, and learns
        // the new one with the answer.
        let asked = page.open();
        assert_eq!(asked, Some((0, 3)));
        page.set_total(4);
        page.fill(&answer(4, asked));
        assert_eq!(lines(&page), [1, 2, 3, 0]);
        let asked = page.open();
        page.fill(&answer(4, asked));
        assert_eq!(lines(&page), [1, 2, 3, 4]);
        assert_eq!(page.waits_for(), 0);
    }

    /// The two pages seen live on 2026-10-12, from one request for three
    /// rows that was never answered.
    #[test]
    fn one_unanswered_request_shifts_the_ranking_page_for_good() {
        let mut page = RankingPage::new();
        page.set_total(3);
        assert_eq!(page.open(), Some((0, 3)));
        // No answer: srp was started again.
        assert_eq!(page.waits_for(), 3);

        // Three players still: the page looks right.
        let asked = page.open();
        page.fill(&answer(3, asked));
        assert_eq!(lines(&page), [1, 2, 3]);
        // A fourth registers. The page is opened twice.
        let asked = page.open();
        page.set_total(4);
        page.fill(&answer(4, asked));
        let asked = page.open();
        page.fill(&answer(4, asked));
        let asked = page.open();
        page.fill(&answer(4, asked));
        // The first screenshot: the fourth player on the first line.
        assert_eq!(lines(&page), [4, 1, 2, 3]);

        // Two more register. The page is opened twice again.
        let asked = page.open();
        page.set_total(6);
        page.fill(&answer(6, asked));
        assert_eq!(lines(&page), [4, 1, 2, 3, 0, 0]);
        let asked = page.open();
        page.fill(&answer(6, asked));
        // The second screenshot: places 4, 5, 6, 3, and two empty lines.
        assert_eq!(lines(&page), [4, 5, 6, 3, 0, 0]);
        assert_eq!(page.waits_for(), 3);
    }

    /// What `push::ranking_flush` carries, as places: the top of the
    /// ranking, then place 1 up to 255 rows.
    fn flush(players: u32) -> Vec<u32> {
        let mut rows: Vec<u32> = (1..=players.min(100)).collect();
        rows.resize(255, 1);
        rows
    }

    #[test]
    fn the_flush_at_a_sign_in_puts_the_ranking_page_right() {
        // The page of the test above, shifted by three rows.
        let mut page = RankingPage::new();
        page.set_total(3);
        page.open();
        for _ in 0..3 {
            let asked = page.open();
            page.set_total(6);
            page.fill(&answer(6, asked));
        }
        assert_ne!(lines(&page), [1, 2, 3, 4, 5, 6]);
        assert_ne!(page.waits_for(), 0);

        // The lobby connection is made again: the count and the flush.
        page.set_total(6);
        page.fill(&flush(6));
        assert_eq!(page.waits_for(), 0);
        // The next time it is opened the page is right, and stays right.
        for _ in 0..2 {
            let asked = page.open();
            assert_eq!(asked, Some((0, 6)));
            page.fill(&answer(6, asked));
            assert_eq!(lines(&page), [1, 2, 3, 4, 5, 6]);
            assert_eq!(page.waits_for(), 0);
        }
    }

    #[test]
    fn the_flush_answers_a_first_page_that_was_open_and_unanswered() {
        // The page is open, its request for the first rows is lost with
        // the connection, and the flush comes with the new sign-in.
        let mut page = RankingPage::new();
        page.set_total(6);
        assert_eq!(page.open(), Some((0, 6)));
        page.fill(&flush(6));
        assert_eq!(lines(&page), [1, 2, 3, 4, 5, 6]);
        assert_eq!(page.waits_for(), 0);
    }

    #[test]
    fn the_flush_does_no_harm_to_a_game_just_started() {
        // Nothing asked yet: every row goes to index 0, the last of them
        // the first place's.
        let mut page = RankingPage::new();
        page.set_total(6);
        page.fill(&flush(6));
        assert_eq!(lines(&page), [1, 0, 0, 0, 0, 0]);
        let asked = page.open();
        page.fill(&answer(6, asked));
        assert_eq!(lines(&page), [1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn only_the_denials_the_client_has_a_window_for_are_shown() {
        let shown: Vec<u8> = (0..=0xFF).filter(|op| denial_is_shown(*op)).collect();
        assert_eq!(shown, [0x2F, 0x30, 0x46, 0x47, 0x48, 0x49]);
    }

    #[test]
    fn the_third_line_in_two_seconds_mutes_the_general_channel_for_ten() {
        let mut filter = ChatSpamFilter::new();
        assert_eq!(filter.send(1_000, "a", channel::GENERAL), Ok(()));
        assert_eq!(filter.send(1_500, "b", channel::GENERAL), Ok(()));
        // The third within 2 s is not sent: muted for 10 s.
        assert_eq!(filter.send(2_999, "c", channel::GENERAL), Err(10_000));
        assert_eq!(filter.send(8_999, "d", channel::GENERAL), Err(4_000));
        assert_eq!(filter.send(12_999, "e", channel::GENERAL), Ok(()));
        // Exactly two seconds apart is not "within".
        let mut filter = ChatSpamFilter::new();
        assert_eq!(filter.send(0, "a", channel::MATCH), Ok(()));
        assert_eq!(filter.send(100, "b", channel::MATCH), Ok(()));
        assert_eq!(filter.send(2_000, "c", channel::MATCH), Ok(()));
        // On a match channel the mute is 5 s.
        assert_eq!(filter.send(2_050, "d", channel::TEAM_2), Err(5_000));
    }

    #[test]
    fn a_long_line_is_not_repeated_within_ten_seconds() {
        let long = "this line has more than twenty characters";
        let mut filter = ChatSpamFilter::new();
        assert_eq!(filter.send(0, long, channel::GENERAL), Ok(()));
        assert_eq!(filter.send(4_000, long, channel::GENERAL), Err(6_000));
        // A short line between forgets it.
        assert_eq!(filter.send(5_000, "ok", channel::GENERAL), Ok(()));
        assert_eq!(filter.send(9_000, long, channel::GENERAL), Ok(()));
        assert_eq!(filter.send(19_000, long, channel::GENERAL), Ok(()));
    }

    #[test]
    fn the_other_channels_have_no_filter() {
        let mut filter = ChatSpamFilter::new();
        for channel in [
            channel::PRIVATE,
            channel::CLAN,
            channel::SQUAD,
            channel::CLAN_TEAM,
        ] {
            assert!(!ChatSpamFilter::applies_to(channel));
            for at in 0..20 {
                assert_eq!(filter.send(at, "x", channel), Ok(()));
            }
        }
    }
}
