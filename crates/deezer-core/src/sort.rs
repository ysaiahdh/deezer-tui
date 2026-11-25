//! Display ordering of track lists.
//!
//! The gateway's `favorite_song.getList` has no ordering parameter and answers
//! oldest-added first, which is the opposite of what Deezer's web player shows.
//! `DATE_ADD` is carried by the entries themselves, so the order is applied
//! here, in memory, and the same helper sorts a cache written by an older build.

use crate::api::models::TrackData;
use crate::config::SortOrder;

/// Reorder `tracks` for display.
///
/// When no track carries a `DATE_ADD` (an endpoint that does not send it, or a
/// cache from before it was read), the API order is used: it is oldest-first,
/// so newest-first is the reverse of it. Tracks with an unknown date (0) end up
/// last in `DateDesc`, first in `DateAsc`; the sort is stable either way, so
/// equal dates keep their API order.
pub fn tracks_by_date(tracks: &mut [TrackData], order: SortOrder) {
    match order {
        SortOrder::Api => {}
        SortOrder::DateDesc if tracks.iter().all(|t| t.date_add == 0) => tracks.reverse(),
        SortOrder::DateDesc => tracks.sort_by_key(|t| std::cmp::Reverse(t.date_add)),
        SortOrder::DateAsc => tracks.sort_by_key(|t| t.date_add),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `TrackData` with only the fields the sorting looks at.
    fn track(id: &str, date_add: u64) -> TrackData {
        serde_json::from_value(serde_json::json!({
            "SNG_ID": id,
            "SNG_TITLE": id,
            "ART_NAME": "Artist",
            "DATE_ADD": date_add,
        }))
        .unwrap()
    }

    fn ids(tracks: &[TrackData]) -> Vec<&str> {
        tracks.iter().map(|t| t.track_id.as_str()).collect()
    }

    /// API order as observed on a real account: oldest added first.
    fn api_order() -> Vec<TrackData> {
        vec![
            track("a", 1_640_863_679),
            track("b", 1_640_863_866),
            track("c", 1_700_000_000),
        ]
    }

    #[test]
    fn date_desc_puts_the_most_recent_first() {
        let mut tracks = api_order();
        tracks_by_date(&mut tracks, SortOrder::DateDesc);
        assert_eq!(ids(&tracks), ["c", "b", "a"]);
    }

    #[test]
    fn date_asc_keeps_the_oldest_first() {
        let mut tracks = api_order();
        tracks_by_date(&mut tracks, SortOrder::DateAsc);
        assert_eq!(ids(&tracks), ["a", "b", "c"]);
    }

    #[test]
    fn api_order_is_left_untouched() {
        let mut tracks = api_order();
        tracks_by_date(&mut tracks, SortOrder::Api);
        assert_eq!(ids(&tracks), ["a", "b", "c"]);
    }

    #[test]
    fn undated_tracks_fall_back_to_reversing_the_api_order() {
        let mut tracks = vec![track("old", 0), track("mid", 0), track("new", 0)];
        tracks_by_date(&mut tracks, SortOrder::DateDesc);
        assert_eq!(ids(&tracks), ["new", "mid", "old"]);

        // Ascending already matches the API order, so nothing moves.
        let mut tracks = vec![track("old", 0), track("mid", 0), track("new", 0)];
        tracks_by_date(&mut tracks, SortOrder::DateAsc);
        assert_eq!(ids(&tracks), ["old", "mid", "new"]);
    }

    #[test]
    fn unknown_dates_sort_last_when_newest_first() {
        let mut tracks = vec![track("old", 10), track("undated", 0), track("new", 20)];
        tracks_by_date(&mut tracks, SortOrder::DateDesc);
        assert_eq!(ids(&tracks), ["new", "old", "undated"]);
    }

    #[test]
    fn equal_dates_keep_their_relative_order() {
        let mut tracks = vec![track("first", 5), track("second", 5), track("third", 5)];
        tracks_by_date(&mut tracks, SortOrder::DateDesc);
        assert_eq!(ids(&tracks), ["first", "second", "third"]);
    }
}
