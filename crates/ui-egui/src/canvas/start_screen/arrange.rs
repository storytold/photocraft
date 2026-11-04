//! Which of the Home screen's recent files are shown, and in what order.

use std::cmp::Ordering;

use photocraft_engine::prefs::HomeRecentSort;

/// One recent file as the Home screen lists it.
pub(super) struct Row {
    pub path: String,
    pub name: String,
    pub folder: String,
    pub kind: String,
    /// When it was last opened (seconds since the epoch): the recorded time, else the file's modified time.
    pub opened: Option<u64>,
    /// Its size, once the file has been looked at.
    pub size: Option<u64>,
}

fn compare(a: &Row, b: &Row, sort: HomeRecentSort) -> Ordering {
    let by_name = || a.name.to_lowercase().cmp(&b.name.to_lowercase()).then_with(|| a.name.cmp(&b.name));
    match sort {
        // Newest first; a file with no time last.
        HomeRecentSort::Recent => b.opened.cmp(&a.opened),
        HomeRecentSort::Name => by_name(),
        // Largest first; a file not looked at yet last.
        HomeRecentSort::Size => b.size.cmp(&a.size),
        HomeRecentSort::Kind => a.kind.cmp(&b.kind).then_with(by_name),
    }
}

/// The indices of `rows` to show: those whose name contains `filter` (case-insensitive, edges
/// trimmed), ordered by `sort` and then flipped when `reverse`. Equal rows keep the list's order.
pub(super) fn arrange(rows: &[Row], sort: HomeRecentSort, reverse: bool, filter: &str) -> Vec<usize> {
    let q = filter.trim().to_lowercase();
    let mut shown: Vec<usize> = (0..rows.len()).filter(|&i| rows.get(i).is_some_and(|r| q.is_empty() || r.name.to_lowercase().contains(&q))).collect();
    shown.sort_by(|&a, &b| {
        let (Some(x), Some(y)) = (rows.get(a), rows.get(b)) else { return Ordering::Equal };
        let order = compare(x, y, sort);
        (if reverse { order.reverse() } else { order }).then(a.cmp(&b))
    });
    shown
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str, kind: &str, opened: Option<u64>, size: Option<u64>) -> Row {
        Row { path: format!("/f/{name}"), name: name.into(), folder: "/f".into(), kind: kind.into(), opened, size }
    }

    fn rows() -> Vec<Row> {
        vec![
            row("coast.psd", "PSD", Some(900), Some(184)),
            row("Brand.pcraft", "PCRAFT", Some(500), Some(12)),
            row("tile.jpg", "JPEG", Some(100), Some(4)),
            row("night.png", "PNG", None, None),
            row("alpine.tif", "TIFF", Some(500), Some(240)),
        ]
    }

    fn names(rows: &[Row], order: &[usize]) -> Vec<String> {
        order.iter().map(|&i| rows[i].name.clone()).collect()
    }

    #[test]
    fn each_sort_orders_the_list_and_reverse_flips_it() {
        let rows = rows();
        let order = |sort, reverse| names(&rows, &arrange(&rows, sort, reverse, ""));
        // Newest first, equal times in list order, no time last.
        assert_eq!(order(HomeRecentSort::Recent, false), ["coast.psd", "Brand.pcraft", "alpine.tif", "tile.jpg", "night.png"]);
        assert_eq!(order(HomeRecentSort::Recent, true), ["night.png", "tile.jpg", "Brand.pcraft", "alpine.tif", "coast.psd"]);
        // Case-insensitive A to Z.
        assert_eq!(order(HomeRecentSort::Name, false), ["alpine.tif", "Brand.pcraft", "coast.psd", "night.png", "tile.jpg"]);
        assert_eq!(order(HomeRecentSort::Name, true), ["tile.jpg", "night.png", "coast.psd", "Brand.pcraft", "alpine.tif"]);
        // Largest first, unknown sizes last.
        assert_eq!(order(HomeRecentSort::Size, false), ["alpine.tif", "coast.psd", "Brand.pcraft", "tile.jpg", "night.png"]);
        assert_eq!(order(HomeRecentSort::Size, true), ["night.png", "tile.jpg", "Brand.pcraft", "coast.psd", "alpine.tif"]);
        // By kind, then name.
        assert_eq!(order(HomeRecentSort::Kind, false), ["tile.jpg", "Brand.pcraft", "night.png", "coast.psd", "alpine.tif"]);
    }

    #[test]
    fn the_filter_is_case_insensitive_on_the_name_and_sort_follows_it() {
        let rows = rows();
        let shown = |filter: &str, sort| names(&rows, &arrange(&rows, sort, false, filter));
        assert_eq!(shown("PSD", HomeRecentSort::Name), ["coast.psd"]);
        assert_eq!(shown("  .T ", HomeRecentSort::Name), ["alpine.tif"]);
        assert_eq!(shown(".p", HomeRecentSort::Name), ["Brand.pcraft", "coast.psd", "night.png"]);
        assert_eq!(shown(".p", HomeRecentSort::Size), ["coast.psd", "Brand.pcraft", "night.png"]);
        // Only the name counts, not the folder or the kind.
        assert!(shown("jpeg", HomeRecentSort::Name).is_empty());
        assert!(shown("/f", HomeRecentSort::Name).is_empty());
        assert!(shown("nothing like it", HomeRecentSort::Recent).is_empty());
        assert_eq!(shown("   ", HomeRecentSort::Name).len(), 5, "a blank filter shows everything");
        assert!(arrange(&[], HomeRecentSort::Recent, true, "x").is_empty());
    }
}
