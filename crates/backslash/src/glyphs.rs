//! Backslash's own icons (`bs-…`): vector paths drawn with GSK in the
//! widget's CSS color, so they are crisp at any scale, look the same on every
//! desktop and take the color of a status, a priority or the text around them.
//!
//! Paths use a 24×24 grid, mostly 2-unit strokes with round ends. A `Fill`
//! shape uses the even-odd rule, which cuts a check or a cross out of a disc.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use gtk::glib;
use gtk::gsk;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Stroke,
    Dashed,
    Fill,
    /// Filled at a third of the opacity: the empty bars of a priority.
    Faint,
}
use Kind::*;

/// A full circle as a path.
macro_rules! circle {
    ($cx:literal, $cy:literal, $r:literal) => {
        // Four quarter arcs from the left point, so only the radius is needed.
        concat!(
            "M", $cx, " ", $cy, "m-", $r, " 0", "a", $r, " ", $r, " 0 0 0 ", $r, " ", $r, "a", $r, " ", $r, " 0 0 0 ",
            $r, " -", $r, "a", $r, " ", $r, " 0 0 0 -", $r, " -", $r, "a", $r, " ", $r, " 0 0 0 -", $r, " ", $r, "z"
        )
    };
}

const ROUNDED_SQUARE: &str = "M6 3h12a3 3 0 0 1 3 3v12a3 3 0 0 1-3 3H6a3 3 0 0 1-3-3V6a3 3 0 0 1 3-3z";

/// Name (without `bs-`) → shapes.
fn shapes(name: &str) -> Option<&'static [(Kind, &'static str)]> {
    Some(match name {
        // Statuses
        "circle" => &[(Stroke, circle!(12, 12, 8))],
        "circle-dashed" => &[(Dashed, circle!(12, 12, 8))],
        "circle-half" => &[(Stroke, circle!(12, 12, 8)), (Fill, "M12 7a5 5 0 0 1 0 10z")],
        "circle-three-quarters" => &[(Stroke, circle!(12, 12, 8)), (Fill, "M12 12V7a5 5 0 1 1-5 5z")],
        "circle-check" => &[(Fill, concat!(circle!(12, 12, 9), "M7.4 12.3l1.5-1.5 2.1 2.1 4.2-4.3 1.5 1.5-5.7 5.8z"))],
        "circle-x" => &[(
            Fill,
            concat!(
                circle!(12, 12, 9),
                "M8.6 10l1.4-1.4 2 2 2-2 1.4 1.4-2 2 2 2-1.4 1.4-2-2-2 2-1.4-1.4 2-2z"
            ),
        )],
        "circle-dot" => &[(Stroke, circle!(12, 12, 8)), (Fill, circle!(12, 12, 3))],
        // Priorities
        "priority-urgent" => &[(Fill, concat!("M6 3h12a3 3 0 0 1 3 3v12a3 3 0 0 1-3 3H6a3 3 0 0 1-3-3V6a3 3 0 0 1 3-3z", "M11 6.5h2v7h-2zM11 15.5h2v2h-2z"))],
        "priority-high" => &[(Fill, "M4 14h4v6H4zM10 9h4v11h-4zM16 4h4v16h-4z")],
        "priority-medium" => &[(Fill, "M4 14h4v6H4zM10 9h4v11h-4z"), (Faint, "M16 4h4v16h-4z")],
        "priority-low" => &[(Fill, "M4 14h4v6H4z"), (Faint, "M10 9h4v11h-4zM16 4h4v16h-4z")],
        "priority-none" => &[(Stroke, "M5 12h2M11 12h2M17 12h2")],
        // Code
        "pr-open" => &[
            (Stroke, circle!(6, 6, 2.2)),
            (Stroke, circle!(6, 18, 2.2)),
            (Stroke, circle!(18, 18, 2.2)),
            (Stroke, "M6 8.2v7.6M18 15.8V9a3 3 0 0 0-3-3h-4M13 3.5 10.5 6 13 8.5"),
        ],
        "pr-draft" => &[
            (Stroke, circle!(6, 6, 2.2)),
            (Stroke, circle!(6, 18, 2.2)),
            (Stroke, circle!(18, 18, 2.2)),
            (Stroke, "M6 8.2v7.6M18 12v.01M18 7v.01"),
        ],
        "pr-merged" => &[
            (Stroke, circle!(6, 6, 2.2)),
            (Stroke, circle!(6, 18, 2.2)),
            (Stroke, circle!(18, 12, 2.2)),
            (Stroke, "M6 8.2v7.6M6 8.2c0 3 2.5 3.8 9.8 3.8"),
        ],
        "pr-closed" => &[
            (Stroke, circle!(6, 6, 2.2)),
            (Stroke, circle!(6, 18, 2.2)),
            (Stroke, circle!(18, 18, 2.2)),
            (Stroke, "M6 8.2v7.6M16 5l4 4M20 5l-4 4M18 12v3.8"),
        ],
        "issue-open" => &[(Stroke, circle!(12, 12, 9)), (Fill, circle!(12, 12, 2))],
        "issue-closed" => &[(Stroke, circle!(12, 12, 9)), (Stroke, "m8.5 12 2.5 2.5 4.5-5")],
        "branch" => &[
            (Stroke, circle!(6, 5, 2.2)),
            (Stroke, circle!(6, 19, 2.2)),
            (Stroke, circle!(18, 7, 2.2)),
            (Stroke, "M6 7.2v9.6M18 9.2v1a4 4 0 0 1-4 4H9a3 3 0 0 0-3 3"),
        ],
        "commit" => &[(Stroke, circle!(12, 12, 3.2)), (Stroke, "M3 12h5.8M15.2 12H21")],
        "repo" => &[(Stroke, "M5 19.5V5a2 2 0 0 1 2-2h12v15H7a2 2 0 0 0-2 2zM5 19.5A2 2 0 0 0 7 21h12")],
        "code" => &[(Stroke, "m8 7-5 5 5 5M16 7l5 5-5 5")],
        "terminal" => &[(Stroke, "m5 7 5 5-5 5M12 18h7")],
        // Things
        "check" => &[(Stroke, "m5 12.5 4.5 4.5L19 7")],
        "x" => &[(Stroke, "M6 6l12 12M18 6 6 18")],
        "dot" => &[(Fill, circle!(12, 12, 4))],
        "star" => &[(Stroke, "M12 3.5l2.6 5.3 5.8.8-4.2 4.1 1 5.8-5.2-2.7-5.2 2.7 1-5.8-4.2-4.1 5.8-.8z")],
        "star-filled" => &[(Fill, "M12 3.5l2.6 5.3 5.8.8-4.2 4.1 1 5.8-5.2-2.7-5.2 2.7 1-5.8-4.2-4.1 5.8-.8z")],
        "heart" => &[(Stroke, "M12 20s-7.5-4.6-7.5-10.2A4.3 4.3 0 0 1 12 7a4.3 4.3 0 0 1 7.5 2.8C19.5 15.4 12 20 12 20z")],
        "heart-filled" => &[(Fill, "M12 20s-7.5-4.6-7.5-10.2A4.3 4.3 0 0 1 12 7a4.3 4.3 0 0 1 7.5 2.8C19.5 15.4 12 20 12 20z")],
        "pin" => &[(Stroke, "M9 4h6l-1 5 3 3v2H7v-2l3-3zM12 14v6")],
        "tag" => &[(Stroke, "M3 12V4h8l10 10-8 8z"), (Fill, circle!(7.5, 8.5, 1.4))],
        "link" => &[(
            Stroke,
            "M10 14a4.5 4.5 0 0 0 6.4 0l3-3a4.5 4.5 0 0 0-6.4-6.4l-1 1M14 10a4.5 4.5 0 0 0-6.4 0l-3 3a4.5 4.5 0 0 0 6.4 6.4l1-1",
        )],
        "comment" => &[(Stroke, "M6 4h12a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2H9l-5 4V6a2 2 0 0 1 2-2z")],
        "user" => &[(Stroke, circle!(12, 8, 4)), (Stroke, "M4.5 20.5c1.4-3.7 4.2-5.5 7.5-5.5s6.1 1.8 7.5 5.5")],
        "calendar" => &[(Stroke, "M6 5h12a2 2 0 0 1 2 2v11a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V7a2 2 0 0 1 2-2zM4 10h16M8 3v4M16 3v4")],
        "clock" => &[(Stroke, circle!(12, 12, 9)), (Stroke, "M12 7v5l3 2")],
        "timer" => &[(Stroke, circle!(12, 13, 8)), (Stroke, "M12 9v4l2.5 1.5M9.5 2.5h5")],
        "bell" => &[(Stroke, "M6 16v-5a6 6 0 0 1 12 0v5l1.5 2h-15zM10 21h4")],
        "folder" => &[(Stroke, "M3 7a2 2 0 0 1 2-2h4l2 2.5h8a2 2 0 0 1 2 2V17a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z")],
        "file" => &[(Stroke, "M7 3h7l5 5v11a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2zM14 3v5h5")],
        "image" => &[
            (Stroke, "M5 4h14a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2zm16 11-5-5-9 9"),
            (Fill, circle!(9, 9.5, 1.7)),
        ],
        "text" => &[(Stroke, "M5 6h14M5 12h14M5 18h9")],
        "globe" => &[(Stroke, circle!(12, 12, 9)), (Stroke, "M3 12h18M12 3c2.8 3 2.8 15 0 18M12 3c-2.8 3-2.8 15 0 18")],
        "lock" => &[(Stroke, "M6 11h12a1 1 0 0 1 1 1v8a1 1 0 0 1-1 1H6a1 1 0 0 1-1-1v-8a1 1 0 0 1 1-1zM8 11V7.5a4 4 0 0 1 8 0V11")],
        "key" => &[(Stroke, circle!(8, 15, 4)), (Stroke, "M10.8 12.2 20 3M17 6l3 3M15 8l2 2")],
        "shield" => &[(Stroke, "M12 3 5 6v5.5c0 4.6 3 7.8 7 9.5 4-1.7 7-4.9 7-9.5V6z")],
        "vpn" => &[(Stroke, "M12 3 5 6v5.5c0 4.6 3 7.8 7 9.5 4-1.7 7-4.9 7-9.5V6zM9.5 12l2 2 3.5-3.5")],
        "bolt" => &[(Stroke, "M13 2 4 14h7l-1 8 9-12h-7z")],
        "sparkle" => &[(
            Stroke,
            "M12 3l1.8 5.2L19 10l-5.2 1.8L12 17l-1.8-5.2L5 10l5.2-1.8zM19 15l.8 2.2 2.2.8-2.2.8-.8 2.2-.8-2.2-2.2-.8 2.2-.8z",
        )],
        "box" | "package" => &[(Stroke, "M12 3l8 4.5v9L12 21l-8-4.5v-9zM4 7.5l8 4.5 8-4.5M12 12v9")],
        "server" => &[(
            Stroke,
            "M5 4h14a1 1 0 0 1 1 1v5H4V5a1 1 0 0 1 1-1zM4 14h16v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1zM8 7v.01M8 17v.01",
        )],
        "cpu" => &[(Stroke, "M7 7h10v10H7zM10 10h4v4h-4zM9 3v4M15 3v4M9 17v4M15 17v4M3 9h4M3 15h4M17 9h4M17 15h4")],
        "plug" => &[(Stroke, "M9 3v5M15 3v5M6 8h12v3a6 6 0 0 1-12 0zM12 17v4")],
        "wifi" => &[(Stroke, "M2.5 9a14 14 0 0 1 19 0M5.5 12.5a9.5 9.5 0 0 1 13 0M8.7 15.8a5 5 0 0 1 6.6 0M12 19.3v.01")],
        "wifi-off" => &[(
            Stroke,
            "M2.5 9a14 14 0 0 1 19 0M5.5 12.5a9.5 9.5 0 0 1 13 0M8.7 15.8a5 5 0 0 1 6.6 0M12 19.3v.01M3 3l18 18",
        )],
        "bluetooth" => &[(Stroke, "m7 7 10 10-5 5V2l5 5L7 17")],
        "speaker" => &[(Stroke, "M4 9h4l5-4v14l-5-4H4zM16.5 8.5a5 5 0 0 1 0 7M19 6a8.5 8.5 0 0 1 0 12")],
        "speaker-off" => &[(Stroke, "M4 9h4l5-4v14l-5-4H4zM16 9l5 6M21 9l-5 6")],
        "headphones" => &[(Stroke, "M4 15v-3a8 8 0 0 1 16 0v3M4 15h3v5H5a1 1 0 0 1-1-1zM20 15h-3v5h2a1 1 0 0 0 1-1z")],
        "music" => &[(Stroke, "M9 18V5l11-2v13M9 18a3 3 0 1 1-6 0a3 3 0 1 1 6 0M20 16a3 3 0 1 1-6 0a3 3 0 1 1 6 0")],
        "mic" => &[(Stroke, "M9 5a3 3 0 0 1 6 0v6a3 3 0 0 1-6 0zM5 11a7 7 0 0 0 14 0M12 18v3")],
        "play" => &[(Fill, "M7 4.5v15l12.5-7.5z")],
        "pause" => &[(Fill, "M6 4h4v16H6zM14 4h4v16h-4z")],
        "stop" => &[(Fill, "M6 6h12v12H6z")],
        "next" => &[(Fill, "M5 5v14l10-7zM16 5h3v14h-3z")],
        "previous" => &[(Fill, "M19 5v14L9 12zM5 5h3v14H5z")],
        "shuffle" => &[(Stroke, "M3 6h3.5c3 0 4.5 1.5 6 4.5s3 4.5 6 4.5H21M3 18h3.5c1.5 0 2.6-.4 3.5-1.2M14 7.2C15 6.4 16.1 6 17.5 6H21M18 3l3 3-3 3M18 12l3 3-3 3")],
        "repeat" => &[(Stroke, "M17 3l3 3-3 3M4 11V9a3 3 0 0 1 3-3h13M7 21l-3-3 3-3M20 13v2a3 3 0 0 1-3 3H4")],
        "window" => &[(Stroke, "M4 5h16a1 1 0 0 1 1 1v12a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V6a1 1 0 0 1 1-1zM3 9h18")],
        "monitor" => &[(Stroke, "M4 4h16a1 1 0 0 1 1 1v10a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1zM8 21h8M12 16v5")],
        "inbox" => &[(Stroke, "M3 13l3-8h12l3 8v6H3zM3 13h5l1.5 2.5h5L16 13h5")],
        "project" => &[(Stroke, "M12 3 21 12 12 21 3 12z")],
        "cycle" => &[(Stroke, "M20 12a8 8 0 1 1-2.3-5.6M20 4v5h-5")],
        "flag" => &[(Stroke, "M5 21V4M5 4h11l-2 4 2 4H5")],
        "alert" => &[(Stroke, "M10.3 4.2 2.6 18a2 2 0 0 0 1.7 3h15.4a2 2 0 0 0 1.7-3L13.7 4.2a2 2 0 0 0-3.4 0zM12 10v4M12 17.5v.01")],
        "info" => &[(Stroke, circle!(12, 12, 9)), (Stroke, "M12 11v5M12 8v.01")],
        "question" => &[(Stroke, circle!(12, 12, 9)), (Stroke, "M9.5 9.5a2.5 2.5 0 1 1 3.5 2.3c-.6.3-1 .9-1 1.6v.6M12 17v.01")],
        "search" => &[(Stroke, circle!(11, 11, 7)), (Stroke, "m20 20-4-4")],
        "plus" => &[(Stroke, "M12 5v14M5 12h14")],
        "minus" => &[(Stroke, "M5 12h14")],
        "refresh" => &[(Stroke, "M20 12a8 8 0 0 1-14.3 4.9M4 12A8 8 0 0 1 18.3 7.1M18.5 3v4.5H14M5.5 21v-4.5H10")],
        "trash" => &[(Stroke, "M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13M10 11v6M14 11v6")],
        "copy" => &[(
            Stroke,
            "M9 9h10a1 1 0 0 1 1 1v10a1 1 0 0 1-1 1H9a1 1 0 0 1-1-1V10a1 1 0 0 1 1-1zM16 9V5a1 1 0 0 0-1-1H5a1 1 0 0 0-1 1v10a1 1 0 0 0 1 1h3",
        )],
        "paste" => &[(Stroke, "M9 3h6v3H9zM9 4.5H6a1 1 0 0 0-1 1V20a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1V5.5a1 1 0 0 0-1-1h-3")],
        "external" => &[(Stroke, "M14 4h6v6M20 4l-9 9M18 14v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h5")],
        "download" => &[(Stroke, "M12 3v12M7 10l5 5 5-5M4 20h16")],
        "upload" => &[(Stroke, "M12 15V3M7 8l5-5 5 5M4 20h16")],
        "settings" => &[(Stroke, "M4 7h10M18 7h2M4 17h4M12 17h8"), (Stroke, circle!(16, 7, 2)), (Stroke, circle!(10, 17, 2))],
        "history" => &[(Stroke, "M3.5 12a8.5 8.5 0 1 0 2.5-6M3 3v5h5M12 8v4.5l3 2")],
        "smile" => &[(Stroke, circle!(12, 12, 9)), (Stroke, "M8.5 14.5a4.5 4.5 0 0 0 7 0M9 9.5v.01M15 9.5v.01")],
        "palette" => &[(
            Stroke,
            "M12 3a9 9 0 1 0 0 18c1.2 0 1.8-.9 1.8-1.8 0-1.3-1-1.7-1-2.9 0-1 .8-1.8 1.8-1.8H17a4 4 0 0 0 4-4C21 6.4 17 3 12 3zM7.5 11v.01M9.5 7.5v.01M14.5 7.5v.01",
        )],
        "sun" => &[(Stroke, circle!(12, 12, 4)), (Stroke, "M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4")],
        "moon" => &[(Stroke, "M20 14.5A8 8 0 1 1 9.5 4a6.5 6.5 0 0 0 10.5 10.5z")],
        "power" => &[(Stroke, "M12 3v9M6.3 6.3a8 8 0 1 0 11.4 0")],
        "mail" => &[(Stroke, "M5 5h14a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7a2 2 0 0 1 2-2zm-2 2 9 6 9-6")],
        "home" => &[(Stroke, "M3 11 12 4l9 7M5 10v10h14V10")],
        "archive" => &[(Stroke, "M3 4h18v4H3zM5 8v12h14V8M10 12h4")],
        "hash" => &[(Stroke, "M5 9h14M5 15h14M10 4 8 20M16 4l-2 16")],
        "filter" => &[(Stroke, "M3 5h18l-7 8v6l-4 2v-8z")],
        "eye" => &[(Stroke, "M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7S2 12 2 12z"), (Stroke, circle!(12, 12, 3))],
        "eye-off" => &[(Stroke, "M10.6 5.1A10 10 0 0 1 12 5c6.5 0 10 7 10 7a17 17 0 0 1-2.2 3M6.6 6.6C3.7 8.4 2 12 2 12s3.5 7 10 7c1.9 0 3.6-.6 5-1.5M9.9 9.9a3 3 0 0 0 4.2 4.2M3 3l18 18")],
        "cloud" => &[(Stroke, "M7 18a4.5 4.5 0 0 1-.5-9A6 6 0 0 1 18 8.5a4.5 4.5 0 0 1-1 9.5z")],
        "keyboard" => &[(
            Stroke,
            "M4 6h16a1 1 0 0 1 1 1v10a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1zM7 10v.01M11 10v.01M15 10v.01M7 14h10",
        )],
        "apps" => &[(Stroke, "M5 4h4a1 1 0 0 1 1 1v4a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1zM15 4h4a1 1 0 0 1 1 1v4a1 1 0 0 1-1 1h-4a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1zM5 14h4a1 1 0 0 1 1 1v4a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1v-4a1 1 0 0 1 1-1zM15 14h4a1 1 0 0 1 1 1v4a1 1 0 0 1-1 1h-4a1 1 0 0 1-1-1v-4a1 1 0 0 1 1-1z")],
        "calculator" => &[(Stroke, "M7 3h10a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2zM8.5 7h7M9 12v.01M12 12v.01M15 12v.01M9 16v.01M12 16v.01M15 16v.01")],
        "arrow-right" => &[(Stroke, "M5 12h14M13 6l6 6-6 6")],
        "arrow-up" => &[(Stroke, "M12 19V5M6 11l6-6 6 6")],
        "arrow-down" => &[(Stroke, "M12 5v14M6 13l6 6 6-6")],
        "chevron-right" => &[(Stroke, "m9 6 6 6-6 6")],
        "chevron-down" => &[(Stroke, "m6 9 6 6 6-6")],
        "back" => &[(Stroke, "m15 6-6 6 6 6")],
        "square" => &[(Stroke, ROUNDED_SQUARE)],
        "square-check" => &[(Fill, concat!("M6 3h12a3 3 0 0 1 3 3v12a3 3 0 0 1-3 3H6a3 3 0 0 1-3-3V6a3 3 0 0 1 3-3z", "M7.4 12.3l1.5-1.5 2.1 2.1 4.2-4.3 1.5 1.5-5.7 5.8z"))],
        _ => return None,
    })
}

pub fn exists(name: &str) -> bool {
    name.strip_prefix("bs-").is_some_and(|n| shapes(n).is_some())
}

thread_local! {
    static PATHS: RefCell<HashMap<String, Vec<(Kind, gsk::Path)>>> = RefCell::default();
}

fn parsed(name: &str) -> Vec<(Kind, gsk::Path)> {
    PATHS.with(|cache| {
        cache
            .borrow_mut()
            .entry(name.to_string())
            .or_insert_with(|| {
                let list = name.strip_prefix("bs-").and_then(shapes).unwrap_or(&[]);
                list.iter()
                    .filter_map(|(kind, d)| match gsk::Path::parse(d) {
                        Ok(path) => Some((*kind, path)),
                        Err(_) => {
                            eprintln!("backslash: bad path in icon {name}");
                            None
                        }
                    })
                    .collect()
            })
            .clone()
    })
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Glyph {
        pub name: RefCell<String>,
        pub size: Cell<i32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Glyph {
        const NAME: &'static str = "BsGlyph";
        type Type = super::Glyph;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_css_name("glyph");
        }
    }

    impl ObjectImpl for Glyph {}

    impl WidgetImpl for Glyph {
        fn measure(&self, _orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            let size = self.size.get();
            (size, size, -1, -1)
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let widget = self.obj();
            let (w, h) = (widget.width() as f32, widget.height() as f32);
            let side = w.min(h);
            if side <= 0.0 {
                return;
            }
            let color = widget.color();
            let faint = gtk::gdk::RGBA::new(color.red(), color.green(), color.blue(), color.alpha() * 0.3);
            let scale = side / 24.0;
            snapshot.save();
            snapshot.translate(&gtk::graphene::Point::new((w - side) / 2.0, (h - side) / 2.0));
            snapshot.scale(scale, scale);
            // At least one device pixel wide, so tiny icons stay visible.
            let stroke = gsk::Stroke::new((2.0f32).max(1.0 / scale));
            stroke.set_line_cap(gsk::LineCap::Round);
            stroke.set_line_join(gsk::LineJoin::Round);
            let dashed = stroke.clone();
            dashed.set_dash(&[3.1, 2.75]);
            for (kind, path) in parsed(&self.name.borrow()) {
                match kind {
                    Stroke => snapshot.append_stroke(&path, &stroke, &color),
                    Dashed => snapshot.append_stroke(&path, &dashed, &color),
                    Fill => snapshot.append_fill(&path, gsk::FillRule::EvenOdd, &color),
                    Faint => snapshot.append_fill(&path, gsk::FillRule::EvenOdd, &faint),
                }
            }
            snapshot.restore();
        }
    }
}

glib::wrapper! {
    pub struct Glyph(ObjectSubclass<imp::Glyph>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Glyph {
    /// `name` is a `bs-…` icon; `size` in pixels.
    pub fn new(name: &str, size: i32) -> Self {
        let glyph: Self = glib::Object::new();
        glyph.imp().name.replace(name.to_string());
        glyph.imp().size.set(size);
        glyph.set_valign(gtk::Align::Center);
        glyph.set_halign(gtk::Align::Center);
        glyph
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_icon_of_the_sdk_exists() {
        let sdk = include_str!("../../../sdk/mod.ts");
        let start = sdk.find("export const Icons = {").expect("Icons in mod.ts");
        let block = &sdk[start..start + sdk[start..].find("} as const").unwrap()];
        let names: Vec<&str> = block.split('"').skip(1).step_by(2).collect();
        assert!(names.len() > 60, "{names:?}");
        for name in names {
            assert!(super::exists(name), "{name} has no shapes");
        }
    }

    #[test]
    fn paths_parse() {
        // gsk needs no display to parse paths.
        for name in ["circle", "circle-check", "priority-urgent", "pr-open", "music", "settings"] {
            for (_, d) in super::shapes(name).unwrap() {
                assert!(gtk::gsk::Path::parse(d).is_ok(), "{name}: {d}");
            }
        }
    }
}
