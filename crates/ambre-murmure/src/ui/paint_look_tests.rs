//! The look of a row, as the mockup of the Murmure calls draws it. The
//! message list spec and the calls spec state it in section 3.

use super::tests::fonts;
use super::*;
use crate::message::{CallEnd, CallView, Mention, MentionTarget, MessageView, ThreadHead};
use crate::ui::list::tests::message;
use ambre_bus::{CallMembers, Char};
use ambre_canvas::theme::LIGHT;
use ambre_core::id::{Call, Document, Segment};
use ambre_core::{Ms, TimeSpan};
use ambre_draw::testing::{DrawOp, RecordingSurface};

/// A day of the tests, in milliseconds.
const DAY: u64 = 86_400_000;

/// The day of `ts` on a calendar whose days start at each multiple of
/// [`DAY`], written by its number.
fn day_of(ts: Instant<Authored>) -> Day {
    let start = ts.get() / DAY * DAY;
    Day {
        start: Instant::<Authored>::new(start),
        written: format!("Day {}", start / DAY),
    }
}

/// The chrome inks of the light theme.
fn inks() -> Inks {
    Inks::of(Theme::Light)
}

/// Every member reads as `Ada Lovelace`.
fn ada(_: Id<User>) -> Option<Validated<ambre_core::DisplayName>> {
    Validated::<ambre_core::DisplayName>::parse("Ada Lovelace").ok()
}

/// Paint `shown` 800 by `height` on a recording surface, each time read
/// as `t`.
fn paint_shown(shown: Shown<'_>, height: f32) -> (RecordingSurface, Painted) {
    let (mut surface, mut font_cx) = (RecordingSurface::new(), fonts());
    let mut cache = TextCache::new();
    let painted = paint_list(
        Pen {
            surface: &mut surface,
            font_cx: &mut font_cx,
            cache: &mut cache,
            palette: &LIGHT,
        },
        shown,
        Size::new(Px::<View>::new(800.0), Px::<View>::new(height)),
        &ada,
        &|_| "t".to_owned(),
    );
    (surface, painted)
}

/// What a list shows with no unread line, no selection and no calendar.
fn plain(list: &MessageList, me: Option<Id<User>>) -> Shown<'_> {
    Shown {
        list,
        unread: None,
        anchor: None,
        focus: None,
        selecting: Selecting::default(),
        me,
        pictures: None,
        theme: Theme::Light,
        calendar: None,
    }
}

/// The glyph runs drawn in `color`: the height of each and its glyphs.
fn runs(surface: &RecordingSurface, color: Color) -> Vec<(f64, usize)> {
    let mut runs: Vec<(f64, usize)> = surface
        .ops()
        .iter()
        .filter_map(|op| match op {
            DrawOp::Glyphs {
                glyph_count,
                color: Some(c),
                transform,
            } if *c == color => Some((transform.translation().y, glyph_count.get() as usize)),
            _ => None,
        })
        .collect();
    runs.sort_by(|a, b| a.0.total_cmp(&b.0));
    runs
}

/// How many paths fill in `color`.
fn filled(surface: &RecordingSurface, color: Color) -> usize {
    surface
        .ops()
        .iter()
        .filter(|op| matches!(op, DrawOp::FillPath { color: Some(c) } if *c == color))
        .count()
}

/// The bounds of each path stroked in `color`.
fn stroked(surface: &RecordingSurface, color: Color) -> Vec<Rect> {
    surface
        .ops()
        .iter()
        .filter_map(|op| match op {
            DrawOp::StrokePath {
                color: Some(c),
                bounds,
                ..
            } if *c == color => Some(*bounds),
            _ => None,
        })
        .collect()
}

/// A list of `messages`, in one segment.
fn list_of(messages: Vec<MessageView>) -> MessageList {
    let mut list = MessageList::default();
    list.absorb(Id::<Segment>::mint(), messages);
    list
}

/// `MURMURE-737`: two messages in a tall viewport paint from its top, and
/// the foot of the viewport stays empty.
#[test]
fn a_short_list_paints_from_the_top_of_the_viewport() {
    let author = Id::<User>::mint();
    let list = list_of(vec![
        message(author, Instant::<Authored>::new(0), "a"),
        message(author, Instant::<Authored>::new(1), "b"),
    ]);
    let (_, painted) = paint_shown(plain(&list, None), 600.0);
    let first = painted.rows.first().expect("a row").bounds;
    let last = painted.rows.last().expect("a row").bounds;
    assert_eq!(first.top, Px::<View>::ZERO);
    assert!(last.bottom < Px::<View>::new(200.0), "{last:?}");
    assert_eq!(painted.latest_shown, OnScreen::Shown);
}

/// `MURMURE-738`: a row that opens a group draws the author's initials in
/// white, on the author's peer colour. A row that joins it draws none.
#[test]
fn a_header_draws_the_initials_of_its_author_on_the_peer_colour() {
    let author = Id::<User>::mint();
    let list = list_of(vec![
        message(author, Instant::<Authored>::new(0), "a"),
        message(author, Instant::<Authored>::new(1), "b"),
    ]);
    let (surface, _) = paint_shown(plain(&list, None), 600.0);
    assert_eq!(filled(&surface, peer_color(&author)), 1, "one avatar");
    let white = Color::from_rgba8(255, 255, 255, 255);
    let initials: usize = runs(&surface, white).iter().map(|r| r.1).sum();
    assert_eq!(initials, 2, "AL");
    let Some(words) = list
        .rows()
        .get(Pos::<ListRow>::ZERO)
        .map(|r| r.words(ada, |_| String::new()))
    else {
        panic!("a row");
    };
    assert_eq!(
        words.header.map(|h| (h.who, h.initials)),
        Some((author, "AL".to_owned()))
    );
}

/// `MURMURE-739`: the divider reads `Today`, `Yesterday` or the date, and
/// stands above the first message of each day alone.
#[test]
fn a_divider_names_the_day_above_its_first_message() {
    let now = Instant::<Authored>::new(10 * DAY + 5);
    let days = Days::of(Calendar {
        now,
        day_of: &day_of,
    });
    let at = |day: u64, ms: u64| Instant::<Authored>::new(day * DAY + ms);
    assert_eq!(days.divider(at(10, 1), None).as_deref(), Some("Today"));
    assert_eq!(
        days.divider(at(9, 1), Some(at(8, 9))).as_deref(),
        Some("Yesterday")
    );
    assert_eq!(days.divider(at(7, 1), None).as_deref(), Some("Day 7"));
    assert_eq!(
        days.divider(at(10, 2), Some(at(10, 1))),
        None,
        "inside a day"
    );

    let author = Id::<User>::mint();
    let list = list_of(vec![
        message(author, at(9, 1), "a"),
        message(author, at(9, 2), "b"),
        message(author, at(10, 1), "c"),
    ]);
    let shown = Shown {
        calendar: Some(Calendar {
            now,
            day_of: &day_of,
        }),
        ..plain(&list, None)
    };
    let (surface, _) = paint_shown(shown, 600.0);
    let labels: Vec<usize> = runs(&surface, inks().faint)
        .iter()
        .map(|r| r.1)
        .filter(|&glyphs| glyphs != 1)
        .collect();
    assert_eq!(labels, ["Yesterday".len(), "Today".len()]);
    let rules =
        surface.count(|op| matches!(op, DrawOp::FillRect { color, .. } if *color == inks().line));
    assert_eq!(rules.get(), 4, "a rule each side of each label");
}

/// `MURMURE-742`: a root shows the faces of its repliers but the member,
/// its count in the accent, and who replied last and how long ago.
#[test]
fn a_root_shows_its_repliers_its_count_and_its_last_reply() {
    let (me, karim, ines) = (Id::<User>::mint(), Id::<User>::mint(), Id::<User>::mint());
    let mut root = message(me, Instant::<Authored>::new(0), "root");
    root.thread = Some(ThreadHead {
        document: Id::<Document>::mint(),
        replies: ambre_core::Count::<crate::message::Replies>::new(4),
        last_replier: Some(karim),
        repliers: vec![karim, me, ines],
        last_reply: Some(Instant::<Authored>::new(DAY)),
    });
    let list = list_of(vec![root]);
    let now = Instant::<Authored>::new(DAY + 5 * 60_000 + 10);
    let shown = Shown {
        calendar: Some(Calendar {
            now,
            day_of: &day_of,
        }),
        ..plain(&list, Some(me))
    };
    let (surface, _) = paint_shown(shown, 600.0);
    assert_eq!(filled(&surface, peer_color(&karim)), 1);
    assert_eq!(filled(&surface, peer_color(&ines)), 1);
    assert_eq!(
        filled(&surface, peer_color(&me)),
        1,
        "the member's own avatar alone"
    );
    let count: usize = runs(&surface, inks().accent).iter().map(|r| r.1).sum();
    assert_eq!(count, "4 replies".chars().count());
    let last: usize = runs(&surface, inks().faint)
        .iter()
        .map(|r| r.1)
        .max()
        .unwrap_or(0);
    assert_eq!(last, "Last reply from Ada 5 min ago".chars().count());
    let words = ThreadWords {
        faces: Vec::new(),
        last: Some(("Karim Bensaid".to_owned(), Instant::<Authored>::new(0))),
    };
    let days = Days::of(Calendar {
        now: Instant::<Authored>::new(5 * 60_000),
        day_of: &day_of,
    });
    assert_eq!(
        last_reply_words(&words, Some(&days)).as_deref(),
        Some("Last reply from Karim 5 min ago")
    );
    assert_eq!(ago(TimeSpan::<Ms>::new(30_000)), "just now");
    assert_eq!(ago(TimeSpan::<Ms>::new(3 * 3_600_000)), "3 h ago");
}

/// `MURMURE-743`: a mention of a member paints in the mention ink, heavy,
/// on a chip of a faint tint of that ink.
#[test]
fn a_mention_paints_as_a_chip_in_the_mention_ink() {
    let (author, named) = (Id::<User>::mint(), Id::<User>::mint());
    let mut sent = message(
        author,
        Instant::<Authored>::new(0),
        "@x are you joining us?",
    );
    sent.mentions = vec![Mention {
        target: MentionTarget::User(named),
        range: Span::<Char>::from_usize_range(0..2),
    }];
    let list = list_of(vec![sent]);
    let (surface, _) = paint_shown(plain(&list, None), 600.0);
    let chip: usize = runs(&surface, inks().mention).iter().map(|r| r.1).sum();
    assert_eq!(chip, "@Ada Lovelace".chars().count());
    assert_eq!(
        filled(
            &surface,
            mix(inks().mention, LIGHT.content_bg, MENTION_GROUND)
        ),
        1
    );
}

/// A live call message with its members.
fn call_message(author: Id<User>, members: Vec<Id<User>>) -> (MessageView, Id<Call>) {
    let call = Id::<Call>::mint();
    let mut sent = message(author, Instant::<Authored>::new(0), "");
    sent.call = Some(CallView {
        call,
        end: CallEnd::Open,
        members: Some(CallMembers::Live(members)),
    });
    (sent, call)
}

/// `MURMURE-745`: a live call message paints a card. It holds a frame, the
/// phone, the title, who started it and the faces in the call. A green
/// `Join` shows while the member is out of it. An ended call shows none.
#[test]
fn a_call_message_paints_as_a_card_with_its_faces_and_join() {
    let (lea, karim, me) = (Id::<User>::mint(), Id::<User>::mint(), Id::<User>::mint());
    let (sent, _) = call_message(lea, vec![lea, karim]);
    let list = list_of(vec![sent.clone()]);
    let (surface, _) = paint_shown(plain(&list, Some(me)), 600.0);
    let frame = mix(inks().ok, inks().line, CARD_BORDER);
    assert_eq!(stroked(&surface, frame).len(), 1, "the frame of the card");
    assert_eq!(
        stroked(&surface, Color::from_rgba8(255, 255, 255, 255)).len(),
        1,
        "the phone"
    );
    assert_eq!(
        filled(&surface, inks().ok),
        2,
        "the icon and the Join button"
    );
    assert_eq!(filled(&surface, peer_color(&karim)), 1, "Karim's face");
    let title: usize = runs(&surface, inks().text).iter().map(|r| r.1).sum();
    assert_eq!(title, "Call in progress".chars().count());
    let started: usize = runs(&surface, inks().dim).iter().map(|r| r.1).sum();
    assert_eq!(started, "Started by Ada \u{b7} t".chars().count());

    let words = list
        .rows()
        .get(Pos::<ListRow>::ZERO)
        .map(|r| r.words(ada, |_| "t".to_owned()));
    let card = words.and_then(|w| w.call).expect("a card");
    assert_eq!(card.started, "Started by Ada \u{b7} t");
    assert!(card.joinable(Some(me)));
    assert!(
        !card.joinable(Some(karim)),
        "a member in the call joins nothing"
    );

    let (in_call, _) = paint_shown(plain(&list, Some(karim)), 600.0);
    assert_eq!(filled(&in_call, inks().ok), 1, "no Join inside the call");
    let mut ended = sent;
    if let Some(view) = ended.call.as_mut() {
        view.end = CallEnd::Written;
    }
    let list = list_of(vec![ended]);
    let (surface, _) = paint_shown(plain(&list, Some(me)), 600.0);
    assert_eq!(
        filled(&surface, inks().ok),
        0,
        "an ended card is muted, with no Join"
    );
    assert_eq!(stroked(&surface, inks().line).len(), 1);
}

/// `MURMURE-746`: a click on `Join` names the call of its card; a click
/// beside it names none.
#[test]
fn a_click_on_join_names_the_call_of_its_card() {
    let (lea, me) = (Id::<User>::mint(), Id::<User>::mint());
    let (sent, call) = call_message(lea, vec![lea]);
    let list = list_of(vec![sent]);
    let (surface, _) = paint_shown(plain(&list, Some(me)), 600.0);
    // The button is the last path the paint fills in the success ink.
    let (mut font_cx, mut cache) = (fonts(), TextCache::new());
    let card = list
        .rows()
        .get(Pos::<ListRow>::ZERO)
        .and_then(|r| r.words(ada, |_| String::new()).call)
        .expect("a card");
    let words = list
        .rows()
        .get(Pos::<ListRow>::ZERO)
        .map(|r| r.words(ada, |_| String::new()))
        .expect("a row");
    let placed = place(
        &mut font_cx,
        &mut cache,
        &words,
        &Above::default(),
        Px::<View>::ZERO,
        Px::<View>::new(800.0),
        None,
    );
    let parts = card_parts(
        &mut font_cx,
        &mut cache,
        &card,
        placed.card.expect("a card"),
        Px::<View>::new(800.0),
        Some(me),
    );
    let button = parts.join.expect("a Join");
    assert_eq!(filled(&surface, inks().ok), 2);
    let click = |x: f64, y: f64| Shown {
        selecting: Selecting {
            click: Some(ambre_core::Point::new(
                Px::<View>::new(x as f32),
                Px::<View>::new(y as f32),
            )),
            ..Selecting::default()
        },
        ..plain(&list, Some(me))
    };
    let middle = button.center();
    let (_, on) = paint_shown(click(middle.x, middle.y), 600.0);
    assert_eq!(on.join, Some((Pos::<ListRow>::ZERO, call)));
    let (_, beside) = paint_shown(click(button.x0 - 30.0, middle.y), 600.0);
    assert_eq!(beside.join, None);
}
