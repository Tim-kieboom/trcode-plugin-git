use trcode_plugin_api::{
    FromPlugin, GitEntry, GitKind, GitSection, GitSummary, Plugin, Refusal, ToPlugin, WireButton,
    WireView,
};

use crate::Viewer;
use crate::viewer::{PAGE_FOR_TESTS, WINDOW_FOR_TESTS};

/// The pane, fed messages one at a time.
struct Pane {
    viewer: Viewer,
    /// What the pane said to the editor last time.
    said: Vec<FromPlugin>,
    rows: Vec<String>,
    selected: Option<usize>,
    buttons: Vec<WireButton>,
    /// The line to type into: its prompt, text, cursor and whether it is active.
    message: (String, String, usize, bool),
}

impl Pane {
    fn new() -> Self {
        let mut pane = Self {
            viewer: Viewer::default(),
            said: Vec::new(),
            rows: Vec::new(),
            selected: None,
            buttons: Vec::new(),
            message: Default::default(),
        };
        pane.hear(ToPlugin::Start);
        pane
    }

    fn hear(&mut self, message: ToPlugin) {
        self.said = self.viewer.handle(message);
        let view = self
            .said
            .iter()
            .rev()
            .find_map(|message| match message {
                FromPlugin::SetView { view } => Some(view.clone()),
                _ => None,
            })
            .expect("every message is answered with a view");
        let WireView::Column { parts } = view else {
            panic!("a column of buttons, rows and the message");
        };
        let [buttons, rows, message] = &parts[..] else {
            panic!("three parts");
        };
        let (WireView::Buttons { buttons }, WireView::Rows { lines, selected }) = (buttons, rows)
        else {
            panic!("buttons and rows");
        };
        let WireView::Input {
            prompt,
            value,
            cursor,
            active,
        } = message
        else {
            panic!("the message is a line to type into");
        };
        self.buttons = buttons.clone();
        self.message = (prompt.clone(), value.clone(), *cursor, *active);
        let (lines, selected) = (lines.clone(), *selected);
        self.rows = lines
            .iter()
            .map(|line| match line.spans.is_empty() {
                true => line.text.clone(),
                false => line.spans.iter().map(|span| span.text.as_str()).collect(),
            })
            .collect();
        self.selected = selected;
    }

    fn act(&mut self, action: &str) {
        self.hear(ToPlugin::action(action));
    }

    /// What it asked for: the pages of files.
    fn pages_asked(&self) -> Vec<(GitSection, usize, u64)> {
        self.said
            .iter()
            .filter_map(|message| match message {
                FromPlugin::GitEntries {
                    request,
                    section,
                    offset,
                    limit,
                } => {
                    assert_eq!(*limit, PAGE_FOR_TESTS);
                    Some((*section, *offset, *request))
                }
                _ => None,
            })
            .collect()
    }

    /// The commands it sent (stage, unstage, open).
    fn commands(&self) -> Vec<&FromPlugin> {
        self.said
            .iter()
            .filter(|message| {
                !matches!(
                    message,
                    FromPlugin::SetView { .. } | FromPlugin::GitEntries { .. }
                )
            })
            .collect()
    }

    fn changed(&mut self, summary: GitSummary) {
        self.hear(ToPlugin::GitChanged { summary });
    }

    /// Answers a request for a page with `entries`, as the editor does.
    fn answer(
        &mut self,
        request: u64,
        generation: u64,
        section: GitSection,
        offset: usize,
        total: usize,
        entries: Vec<GitEntry>,
    ) {
        self.hear(ToPlugin::GitEntries {
            request,
            generation,
            section,
            offset,
            total,
            entries,
        });
    }
}

fn summary(generation: u64, staged: usize, changes: usize, untracked: usize) -> GitSummary {
    let mut summary = GitSummary::empty(generation);
    summary.branch = Some("main".into());
    summary.commit = Some("abc1234def".into());
    summary.staged = staged;
    summary.changes = changes;
    summary.untracked = untracked;
    summary
}

fn modified(path: &str) -> GitEntry {
    GitEntry::new(path, GitKind::Modified)
}

/// A pane that has been told about `changes` modified files and has them.
fn with_changes(paths: &[&str]) -> Pane {
    let mut pane = Pane::new();
    pane.changed(summary(1, 0, paths.len(), 0));
    let (_, _, request) = pane.pages_asked()[0];
    let entries = paths.iter().map(|path| modified(path)).collect();
    pane.answer(request, 1, GitSection::Changes, 0, paths.len(), entries);
    pane
}

// --- what it shows ---

#[test]
fn before_it_hears_anything_it_waits() {
    let pane = Pane::new();

    assert_eq!(pane.rows, vec!["…"]);
    assert_eq!(pane.selected, None);
}

#[test]
fn without_a_repository_it_says_so() {
    let mut pane = Pane::new();

    pane.hear(ToPlugin::GitUnavailable);

    assert_eq!(pane.rows, vec!["No git repository"]);
}

#[test]
fn a_repository_with_nothing_changed_says_no_changes_under_the_branch() {
    let mut pane = Pane::new();

    pane.changed(summary(1, 0, 0, 0));

    assert_eq!(pane.rows, vec!["main", "No changes"]);
    assert_eq!(pane.selected, None);
    assert!(pane.pages_asked().is_empty(), "nothing to ask for");
}

#[test]
fn the_branch_row_says_how_far_it_is_from_the_branch_it_follows() {
    let mut pane = Pane::new();
    let mut ahead = summary(1, 0, 0, 0);
    ahead.ahead = 2;
    ahead.behind = 5;
    pane.changed(ahead);
    assert_eq!(pane.rows[0], "main ↑2 ↓5");

    let mut detached = summary(2, 0, 0, 0);
    detached.branch = None;
    pane.changed(detached);
    assert_eq!(pane.rows[0], "(abc1234)");

    let mut unborn = summary(3, 0, 0, 0);
    unborn.branch = None;
    unborn.commit = None;
    pane.changed(unborn);
    assert_eq!(pane.rows[0], "(no commit)");
}

#[test]
fn the_files_are_asked_for_when_the_summary_comes_and_shown_when_they_do() {
    let mut pane = Pane::new();

    pane.changed(summary(1, 0, 2, 0));

    let asked = pane.pages_asked();
    assert_eq!(asked.len(), 1);
    assert_eq!((asked[0].0, asked[0].1), (GitSection::Changes, 0));
    assert_eq!(pane.rows, vec!["main", "Changes (2)", "  …", "  …"]);

    pane.answer(
        asked[0].2,
        1,
        GitSection::Changes,
        0,
        2,
        vec![modified("src/a.rs"), modified("b.rs")],
    );

    assert_eq!(
        pane.rows,
        vec!["main", "Changes (2)", "M a.rs  src/", "M b.rs"]
    );
    assert_eq!(pane.selected, Some(2), "the first file is selected");
    assert!(pane.pages_asked().is_empty(), "nothing more to ask for");
}

#[test]
fn the_groups_are_in_the_order_of_a_git_pane_and_the_empty_ones_are_left_out() {
    let mut pane = Pane::new();

    pane.changed(summary(1, 1, 0, 2));

    assert_eq!(
        pane.rows,
        vec![
            "main",
            "Staged Changes (1)",
            "  …",
            "Untracked (2)",
            "  …",
            "  …"
        ]
    );
    let sections: Vec<_> = pane.pages_asked().iter().map(|asked| asked.0).collect();
    assert_eq!(sections, vec![GitSection::Staged, GitSection::Untracked]);
}

#[test]
fn conflicts_come_first() {
    let mut pane = Pane::new();
    let mut with_conflict = summary(1, 1, 1, 1);
    with_conflict.conflicts = 1;

    pane.changed(with_conflict);

    assert_eq!(pane.rows[1], "Merge Conflicts (1)");
    assert_eq!(pane.rows[3], "Staged Changes (1)");
}

#[test]
fn each_kind_of_change_has_its_letter_and_a_renamed_file_says_where_it_came_from() {
    let mut pane = Pane::new();
    pane.changed(summary(1, 4, 0, 0));
    let request = pane.pages_asked()[0].2;

    pane.answer(
        request,
        1,
        GitSection::Staged,
        0,
        4,
        vec![
            GitEntry::new("a.rs", GitKind::Added),
            GitEntry::new("d.rs", GitKind::Deleted),
            GitEntry::new("new.rs", GitKind::Renamed).renamed_from("old.rs"),
            GitEntry::new("t.rs", GitKind::TypeChanged),
        ],
    );

    assert_eq!(
        pane.rows[2..],
        ["A a.rs", "D d.rs", "R new.rs  ← old.rs", "T t.rs"]
    );
}

#[test]
fn the_parts_of_a_row_have_the_styles_of_their_kind() {
    let mut pane = with_changes(&["src/a.rs"]);
    pane.hear(ToPlugin::Refresh);
    let Some(FromPlugin::SetView {
        view: WireView::Column { parts },
    }) = pane.said.last().cloned()
    else {
        panic!("a column");
    };
    let WireView::Rows { lines, .. } = &parts[1] else {
        panic!("rows");
    };

    let spans = &lines[2].spans;

    assert_eq!(spans[0].slot, "git.modified");
    assert_eq!(spans[1].slot, "explorer.file");
    assert_eq!(spans[2].slot, "explorer.ignored", "the folder is dimmed");
    assert_eq!(lines[0].slot, "explorer.active");
    assert_eq!(lines[1].slot, "explorer.dir");
}

// --- moving ---

#[test]
fn up_and_down_move_the_selected_file_and_stop_at_the_ends() {
    let mut pane = with_changes(&["a.rs", "b.rs", "c.rs"]);

    pane.act("up");
    assert_eq!(pane.selected, Some(2), "at the first file already");
    pane.act("down");
    assert_eq!(pane.selected, Some(3));
    pane.act("down");
    pane.act("down");
    assert_eq!(pane.selected, Some(4), "at the last file");
}

#[test]
fn the_selection_goes_from_one_group_to_the_next() {
    let mut pane = Pane::new();
    pane.changed(summary(1, 1, 1, 0));
    let asked = pane.pages_asked();
    pane.answer(
        asked[0].2,
        1,
        GitSection::Staged,
        0,
        1,
        vec![modified("s.rs")],
    );
    pane.answer(
        asked[1].2,
        1,
        GitSection::Changes,
        0,
        1,
        vec![modified("c.rs")],
    );
    assert_eq!(pane.rows[pane.selected.unwrap()], "M s.rs");

    pane.act("down");

    assert_eq!(pane.rows[pane.selected.unwrap()], "M c.rs");
}

#[test]
fn a_new_summary_with_fewer_files_keeps_the_selection_inside() {
    let mut pane = with_changes(&["a.rs", "b.rs", "c.rs"]);
    pane.act("down");
    pane.act("down");

    pane.changed(summary(2, 0, 1, 0));

    assert_eq!(pane.rows, vec!["main", "Changes (1)", "  …"]);
    assert_eq!(pane.selected, Some(2));
}

// --- commands ---

fn selected_path(pane: &Pane) -> String {
    pane.rows[pane.selected.unwrap()].clone()
}

#[test]
fn open_asks_the_editor_to_open_the_selected_file() {
    let mut pane = with_changes(&["src/a.rs", "b.rs"]);
    pane.act("down");

    pane.act("open");

    assert_eq!(selected_path(&pane), "M b.rs");
    assert!(matches!(
        pane.commands()[..],
        [FromPlugin::OpenFile { path, .. }] if path == "b.rs"
    ));
}

#[test]
fn a_deleted_file_and_a_folder_of_untracked_files_cannot_be_opened_and_it_says_why() {
    let mut pane = Pane::new();
    pane.changed(summary(1, 0, 1, 1));
    let asked = pane.pages_asked();
    pane.answer(
        asked[0].2,
        1,
        GitSection::Changes,
        0,
        1,
        vec![GitEntry::new("gone.rs", GitKind::Deleted)],
    );
    pane.answer(
        asked[1].2,
        1,
        GitSection::Untracked,
        0,
        1,
        vec![GitEntry::new("build/", GitKind::Untracked)],
    );

    pane.act("open");
    assert!(pane.commands().is_empty());
    assert_eq!(
        pane.rows.last().unwrap(),
        "nothing to open: the file is deleted"
    );

    pane.act("down");
    pane.act("open");
    assert!(pane.commands().is_empty());
    assert_eq!(
        pane.rows.last().unwrap(),
        "nothing to open: a folder of untracked files"
    );
}

#[test]
fn stage_stages_a_changed_file_and_refuses_one_that_is_staged() {
    let mut pane = with_changes(&["a.rs"]);
    pane.act("stage");
    assert!(matches!(
        pane.commands()[..],
        [FromPlugin::GitStage { path, .. }] if path == "a.rs"
    ));

    let mut staged = Pane::new();
    staged.changed(summary(1, 1, 0, 0));
    let request = staged.pages_asked()[0].2;
    staged.answer(request, 1, GitSection::Staged, 0, 1, vec![modified("a.rs")]);
    staged.act("stage");
    assert!(staged.commands().is_empty());
    assert_eq!(staged.rows.last().unwrap(), "already staged");
}

#[test]
fn unstage_unstages_a_staged_file_and_refuses_one_that_is_not() {
    let mut staged = Pane::new();
    staged.changed(summary(1, 1, 0, 0));
    let request = staged.pages_asked()[0].2;
    staged.answer(request, 1, GitSection::Staged, 0, 1, vec![modified("a.rs")]);
    staged.act("unstage");
    assert!(matches!(
        staged.commands()[..],
        [FromPlugin::GitUnstage { path, .. }] if path == "a.rs"
    ));

    let mut pane = with_changes(&["a.rs"]);
    pane.act("unstage");
    assert!(pane.commands().is_empty());
    assert_eq!(pane.rows.last().unwrap(), "not staged");
}

#[test]
fn an_untracked_file_is_staged_too() {
    let mut pane = Pane::new();
    pane.changed(summary(1, 0, 0, 1));
    let request = pane.pages_asked()[0].2;
    pane.answer(
        request,
        1,
        GitSection::Untracked,
        0,
        1,
        vec![GitEntry::new("new.rs", GitKind::Untracked)],
    );

    pane.act("stage");

    assert!(matches!(pane.commands()[..], [FromPlugin::GitStage { .. }]));
}

#[test]
fn commands_with_nothing_selected_do_nothing() {
    let mut pane = Pane::new();
    pane.changed(summary(1, 0, 0, 0));

    for action in ["open", "stage", "unstage", "up", "down"] {
        pane.act(action);
        assert!(pane.commands().is_empty(), "{action}");
    }
}

#[test]
fn a_click_on_a_file_selects_and_opens_it_and_a_click_elsewhere_does_nothing() {
    let mut pane = with_changes(&["a.rs", "b.rs"]);

    pane.hear(ToPlugin::Click { row: 3 });
    assert_eq!(pane.selected, Some(3));
    assert!(matches!(
        pane.commands()[..],
        [FromPlugin::OpenFile { path, .. }] if path == "b.rs"
    ));

    for row in [0, 1, 99] {
        pane.hear(ToPlugin::Click { row });
        assert!(pane.commands().is_empty(), "row {row}");
    }
}

#[test]
fn what_the_editor_refuses_is_shown_under_the_files_until_the_next_command() {
    let mut pane = with_changes(&["a.rs"]);
    pane.act("stage");
    let request = match pane.commands()[0] {
        FromPlugin::GitStage { request, .. } => *request,
        other => panic!("{other:?}"),
    };

    pane.hear(ToPlugin::Refused {
        request,
        reason: Refusal::Failed {
            detail: "fatal: Unable to create index.lock".into(),
        },
    });
    assert_eq!(
        pane.rows.last().unwrap(),
        "fatal: Unable to create index.lock"
    );

    pane.act("down");
    assert_ne!(
        pane.rows.last().unwrap(),
        "fatal: Unable to create index.lock"
    );
}

#[test]
fn a_capability_the_plugin_lacks_is_said_by_name() {
    let mut pane = with_changes(&["a.rs"]);

    pane.hear(ToPlugin::Refused {
        request: 99,
        reason: Refusal::Denied {
            capability: trcode_plugin_api::Capability::GitWrite,
        },
    });

    assert_eq!(pane.rows.last().unwrap(), "not allowed: git.write");
}

#[test]
fn a_page_that_is_refused_is_not_asked_for_again_and_again() {
    let mut pane = Pane::new();
    pane.changed(summary(1, 0, 3, 0));
    let request = pane.pages_asked()[0].2;

    pane.hear(ToPlugin::Refused {
        request,
        reason: Refusal::NoRepository,
    });

    assert!(pane.pages_asked().is_empty());
    pane.act("down");
    assert!(pane.pages_asked().is_empty());
    // A new state starts over.
    pane.changed(summary(2, 0, 3, 0));
    assert_eq!(pane.pages_asked().len(), 1);
}

// --- states and pages ---

#[test]
fn a_page_of_another_state_is_dropped_and_the_new_summary_asks_again() {
    let mut pane = Pane::new();
    pane.changed(summary(1, 0, 2, 0));
    let old = pane.pages_asked()[0].2;
    pane.changed(summary(2, 0, 2, 0));
    let new = pane.pages_asked()[0].2;
    assert_ne!(old, new);

    pane.answer(old, 1, GitSection::Changes, 0, 2, vec![modified("old.rs")]);
    assert_eq!(pane.rows[2], "  …", "the old page is not shown");

    pane.answer(
        new,
        2,
        GitSection::Changes,
        0,
        2,
        vec![modified("new.rs"), modified("b.rs")],
    );
    assert_eq!(pane.rows[2], "M new.rs");
}

#[test]
fn a_page_stamped_with_a_generation_the_pane_does_not_have_is_dropped() {
    let mut pane = Pane::new();
    pane.changed(summary(1, 0, 1, 0));
    let request = pane.pages_asked()[0].2;

    pane.answer(
        request,
        7,
        GitSection::Changes,
        0,
        1,
        vec![modified("x.rs")],
    );

    assert_eq!(pane.rows[2], "  …");
}

#[test]
fn an_answer_to_nothing_that_was_asked_is_ignored() {
    let mut pane = with_changes(&["a.rs"]);

    pane.answer(
        12345,
        1,
        GitSection::Changes,
        0,
        1,
        vec![modified("zzz.rs")],
    );

    assert_eq!(pane.rows[2], "M a.rs");
}

#[test]
fn refresh_asks_for_the_files_again() {
    let mut pane = with_changes(&["a.rs"]);
    assert!(pane.pages_asked().is_empty());

    pane.act("refresh");

    assert_eq!(pane.pages_asked().len(), 1);
    assert_eq!(pane.rows[2], "  …");
}

#[test]
fn a_summary_of_the_same_state_after_a_restart_is_taken_like_any() {
    let mut pane = with_changes(&["a.rs"]);

    pane.changed(summary(1, 0, 1, 0));

    assert_eq!(pane.pages_asked().len(), 1);
}

// --- big repositories ---

fn many(count: usize) -> Pane {
    let mut pane = Pane::new();
    pane.changed(summary(1, 0, count, 0));
    pane
}

fn names(from: usize, to: usize) -> Vec<GitEntry> {
    (from..to)
        .map(|n| modified(&format!("f{n:05}.rs")))
        .collect()
}

#[test]
fn only_the_first_page_is_asked_for_at_the_start_however_many_files_there_are() {
    let pane = many(10_000);

    let asked = pane.pages_asked();

    assert_eq!(asked.len(), 1, "{asked:?}");
    assert_eq!(asked[0].1, 0);
    assert!(
        pane.rows.len() <= WINDOW_FOR_TESTS + 2,
        "the branch, the heading and a window of the list"
    );
}

#[test]
fn moving_down_through_a_big_list_asks_for_the_next_page_when_it_comes_into_the_window() {
    let mut pane = many(10_000);
    let request = pane.pages_asked()[0].2;
    pane.answer(
        request,
        1,
        GitSection::Changes,
        0,
        10_000,
        names(0, PAGE_FOR_TESTS),
    );
    let mut asked_later = Vec::new();

    for _ in 0..WINDOW_FOR_TESTS {
        pane.act("down");
        asked_later.extend(pane.pages_asked().into_iter().map(|asked| asked.1));
    }

    assert_eq!(asked_later, vec![PAGE_FOR_TESTS], "the second page, once");
}

#[test]
fn the_rows_are_a_window_around_the_selected_file() {
    let mut pane = many(1_000);
    let request = pane.pages_asked()[0].2;
    pane.answer(
        request,
        1,
        GitSection::Changes,
        0,
        1_000,
        names(0, PAGE_FOR_TESTS),
    );
    for _ in 0..500 {
        pane.act("down");
    }

    let asked: Vec<usize> = pane.pages_asked().iter().map(|asked| asked.1).collect();

    assert!(pane.rows.len() <= WINDOW_FOR_TESTS + 2);
    let selected = pane.selected.unwrap();
    assert!(selected > 1 && selected < pane.rows.len(), "{selected}");
    assert_eq!(pane.rows[selected], "  …", "its page has not come yet");
    assert!(asked.iter().all(|offset| *offset >= 400), "{asked:?}");
}

#[test]
fn a_window_over_two_groups_asks_for_both() {
    let mut pane = Pane::new();
    pane.changed(summary(1, 5, 5, 0));

    let sections: Vec<_> = pane.pages_asked().iter().map(|asked| asked.0).collect();

    assert_eq!(sections, vec![GitSection::Staged, GitSection::Changes]);
}

// --- the shape of what it says ---

#[test]
fn every_message_is_answered_with_a_view_last() {
    let mut pane = Pane::new();

    for message in [
        ToPlugin::Refresh,
        ToPlugin::Unknown,
        ToPlugin::text("x"),
        ToPlugin::InputEnded,
        ToPlugin::Done { request: 1 },
    ] {
        pane.hear(message);
        assert!(matches!(pane.said.last(), Some(FromPlugin::SetView { .. })));
    }
}

#[test]
fn the_selected_row_is_always_a_row_that_exists() {
    let mut pane = with_changes(&["a.rs", "b.rs"]);

    for action in ["down", "down", "down", "up", "up", "up", "up", "refresh"] {
        pane.act(action);
        if let Some(selected) = pane.selected {
            assert!(selected < pane.rows.len(), "{action}");
        }
    }
}

// --- the buttons, the message and the commands to the remote ---

impl Pane {
    fn button(&self, id: &str) -> &WireButton {
        self.buttons
            .iter()
            .find(|button| button.id == id)
            .unwrap_or_else(|| panic!("no button {id}: {:?}", self.buttons))
    }

    fn has_button(&self, id: &str) -> bool {
        self.buttons.iter().any(|button| button.id == id)
    }

    fn press(&mut self, id: &str) {
        self.hear(ToPlugin::Press { button: id.into() });
    }

    fn text(&mut self, text: &str) {
        self.hear(ToPlugin::text(text));
    }

    fn key(&mut self, key: &str) {
        self.hear(ToPlugin::key(key, false, false, false));
    }

    fn typed(&self) -> &str {
        &self.message.1
    }

    fn cursor(&self) -> usize {
        self.message.2
    }

    fn typing(&self) -> bool {
        self.message.3
    }

    /// The requests of one kind it sent, by their message.
    fn sent(&self) -> Vec<&FromPlugin> {
        self.said
            .iter()
            .filter(|message| !matches!(message, FromPlugin::SetView { .. }))
            .collect()
    }
}

fn summary_of(
    generation: u64,
    staged: usize,
    changes: usize,
    upstream: Option<&str>,
    ahead: u32,
    behind: u32,
) -> GitSummary {
    let mut summary = summary(generation, staged, changes, 0);
    summary.branch = Some("main".into());
    summary.upstream = upstream.map(str::to_owned);
    summary.ahead = ahead;
    summary.behind = behind;
    summary
}

fn pane_with(summary: GitSummary) -> Pane {
    let mut pane = Pane::new();
    pane.changed(summary);
    pane
}

#[test]
fn the_buttons_are_in_the_order_of_the_layout_and_the_message_is_a_line_under_them() {
    let pane = pane_with(summary_of(1, 1, 1, Some("origin/main"), 0, 0));

    let ids: Vec<&str> = pane.buttons.iter().map(|b| b.id.as_str()).collect();

    assert_eq!(
        ids,
        [
            "commit",
            "stage_all",
            "unstage_all",
            "fetch",
            "pull",
            "push"
        ]
    );
    assert_eq!(pane.message.0, "Message: ");
    assert!(!pane.typing(), "it is not typed into until asked");
}

#[test]
fn only_what_can_be_done_can_be_pressed() {
    // Nothing changed and nothing to push: only fetch and pull.
    let pane = pane_with(summary_of(1, 0, 0, Some("origin/main"), 0, 0));
    for (id, enabled) in [
        ("commit", false),
        ("stage_all", false),
        ("unstage_all", false),
        ("fetch", true),
        ("pull", true),
        ("push", false),
    ] {
        assert_eq!(pane.button(id).enabled, enabled, "{id}");
    }

    // Something staged and something not, and a commit to push.
    let pane = pane_with(summary_of(1, 2, 3, Some("origin/main"), 1, 0));
    for id in [
        "commit",
        "stage_all",
        "unstage_all",
        "fetch",
        "pull",
        "push",
    ] {
        assert!(pane.button(id).enabled, "{id}");
    }
}

#[test]
fn a_branch_with_no_upstream_can_be_pushed_but_not_pulled() {
    let pane = pane_with(summary_of(1, 0, 0, None, 0, 0));

    assert!(
        pane.button("push").enabled,
        "the first push sets the upstream"
    );
    assert!(!pane.button("pull").enabled);
}

#[test]
fn a_detached_head_cannot_be_pushed() {
    let mut detached = summary_of(1, 0, 0, None, 0, 0);
    detached.branch = None;
    let pane = pane_with(detached);

    assert!(!pane.button("push").enabled);
}

#[test]
fn how_far_the_branch_is_from_its_upstream_is_on_the_buttons() {
    let pane = pane_with(summary_of(1, 0, 0, Some("origin/main"), 2, 3));

    assert_eq!(pane.button("pull").label, "Pull (3)");
    assert_eq!(pane.button("push").label, "Push (2)");
    let level = pane_with(summary_of(1, 0, 0, Some("origin/main"), 0, 0));
    assert_eq!(level.button("pull").label, "Pull");
    assert_eq!(level.button("push").label, "Push");
}

#[test]
fn without_a_repository_nothing_can_be_pressed() {
    let mut pane = Pane::new();
    pane.hear(ToPlugin::GitUnavailable);

    assert!(pane.buttons.iter().all(|button| !button.enabled));
}

#[test]
fn stage_all_and_unstage_all_send_their_requests_and_refuse_when_there_is_nothing_to_do() {
    let mut pane = pane_with(summary_of(1, 0, 2, None, 0, 0));
    pane.press("stage_all");
    assert!(matches!(pane.sent()[..], [FromPlugin::GitStageAll { .. }]));
    pane.press("unstage_all");
    assert!(pane.sent().is_empty(), "nothing is staged");
    assert!(pane.rows.last().unwrap().contains("nothing is staged"));

    let mut pane = pane_with(summary_of(1, 2, 0, None, 0, 0));
    pane.press("unstage_all");
    assert!(matches!(
        pane.sent()[..],
        [FromPlugin::GitUnstageAll { .. }]
    ));
    pane.press("stage_all");
    assert!(pane.sent().is_empty(), "nothing is left to stage");
}

#[test]
fn the_keys_of_the_actions_do_the_same_as_the_buttons() {
    let mut pane = pane_with(summary_of(1, 1, 1, Some("origin/main"), 1, 1));
    pane.act("stage_all");
    assert!(matches!(pane.sent()[..], [FromPlugin::GitStageAll { .. }]));
    pane.act("unstage_all");
    assert!(matches!(
        pane.sent()[..],
        [FromPlugin::GitUnstageAll { .. }]
    ));
}

// --- the message ---

#[test]
fn the_commit_action_moves_to_the_message_and_typing_goes_in_it() {
    let mut pane = pane_with(summary_of(1, 1, 0, None, 0, 0));
    pane.act("commit");
    assert!(pane.typing());

    pane.text("Fix the thing");

    assert_eq!(pane.typed(), "Fix the thing");
    assert_eq!(pane.cursor(), 13);
}

#[test]
fn text_is_ignored_while_the_message_is_not_being_typed_into() {
    let mut pane = pane_with(summary_of(1, 1, 0, None, 0, 0));

    pane.text("stray");
    pane.key("backspace");

    assert_eq!(pane.typed(), "");
}

#[test]
fn the_message_is_edited_at_the_cursor() {
    let mut pane = pane_with(summary_of(1, 1, 0, None, 0, 0));
    pane.act("commit");
    pane.text("helo wrld");
    pane.key("home");
    for _ in 0..3 {
        pane.key("right");
    }
    pane.text("l");
    assert_eq!(pane.typed(), "hello wrld");
    assert_eq!(pane.cursor(), 4);

    pane.key("end");
    pane.key("left");
    pane.key("left");
    pane.key("left");
    pane.text("o");
    assert_eq!(pane.typed(), "hello world");

    pane.key("backspace");
    assert_eq!(pane.typed(), "hello wrld");
    pane.key("delete");
    assert_eq!(pane.typed(), "hello wld");
    pane.hear(ToPlugin::key("u", true, false, false));
    assert_eq!(pane.typed(), "");
    assert_eq!(pane.cursor(), 0);
}

#[test]
fn editing_the_ends_of_the_message_does_nothing_wrong() {
    let mut pane = pane_with(summary_of(1, 1, 0, None, 0, 0));
    pane.act("commit");
    pane.key("backspace");
    pane.key("delete");
    pane.key("left");
    assert_eq!((pane.typed(), pane.cursor()), ("", 0));
    pane.text("ab");
    pane.key("delete");
    pane.key("right");
    assert_eq!((pane.typed(), pane.cursor()), ("ab", 2));
}

#[test]
fn a_message_with_wide_characters_is_edited_by_character() {
    let mut pane = pane_with(summary_of(1, 1, 0, None, 0, 0));
    pane.act("commit");
    pane.text("a世é");
    assert_eq!(pane.cursor(), 3);

    pane.key("left");
    pane.key("backspace");

    assert_eq!(pane.typed(), "aé");
    assert_eq!(pane.cursor(), 1);
}

#[test]
fn a_pasted_line_break_is_a_space() {
    let mut pane = pane_with(summary_of(1, 1, 0, None, 0, 0));
    pane.act("commit");
    pane.text("one\r\ntwo\nthree");
    assert_eq!(pane.typed(), "one  two three");
}

#[test]
fn a_click_in_the_message_makes_it_the_line_typed_into_from_that_character() {
    let mut pane = pane_with(summary_of(1, 1, 0, None, 0, 0));
    pane.act("commit");
    pane.text("hello");
    pane.hear(ToPlugin::InputEnded);
    assert!(!pane.typing());

    pane.hear(ToPlugin::InputClick { column: 2 });

    assert!(pane.typing());
    assert_eq!(pane.cursor(), 2);
    pane.hear(ToPlugin::InputClick { column: 99 });
    assert_eq!(pane.cursor(), 5, "past the end is the end");
}

#[test]
fn esc_ends_the_typing_and_keeps_the_text() {
    let mut pane = pane_with(summary_of(1, 1, 0, None, 0, 0));
    pane.act("commit");
    pane.text("keep me");

    pane.hear(ToPlugin::InputEnded);

    assert!(!pane.typing());
    assert_eq!(pane.typed(), "keep me");
}

// --- commit ---

#[test]
fn the_commit_button_with_no_message_asks_for_one_and_sends_nothing() {
    let mut pane = pane_with(summary_of(1, 1, 0, None, 0, 0));

    pane.press("commit");

    assert!(pane.sent().is_empty());
    assert!(pane.typing());
    assert!(pane.rows.last().unwrap().contains("type a commit message"));
}

#[test]
fn the_commit_button_and_enter_in_the_message_commit_with_what_was_typed() {
    for finish in [
        (|pane: &mut Pane| pane.press("commit")) as fn(&mut Pane),
        |pane: &mut Pane| pane.key("enter"),
    ] {
        let mut pane = pane_with(summary_of(1, 1, 0, None, 0, 0));
        pane.act("commit");
        pane.text("Add the thing");

        finish(&mut pane);

        let [FromPlugin::GitCommit { message, .. }] = pane.sent()[..] else {
            panic!("a commit: {:?}", pane.sent());
        };
        assert_eq!(message, "Add the thing");
    }
}

#[test]
fn a_commit_that_is_done_clears_the_message_and_stops_the_typing() {
    let mut pane = pane_with(summary_of(1, 1, 0, None, 0, 0));
    pane.act("commit");
    pane.text("Add the thing");
    pane.key("enter");
    let request = pane.answer_request_of_commit();

    pane.hear(ToPlugin::Done { request });

    assert_eq!(pane.typed(), "");
    assert!(!pane.typing());
}

#[test]
fn a_commit_that_is_refused_keeps_the_message_and_says_why() {
    let mut pane = pane_with(summary_of(1, 1, 0, None, 0, 0));
    pane.act("commit");
    pane.text("Add the thing");
    pane.key("enter");
    let request = pane.answer_request_of_commit();

    pane.hear(ToPlugin::Refused {
        request,
        reason: Refusal::Failed {
            detail: "lint: tabs are not allowed".into(),
        },
    });

    assert_eq!(pane.typed(), "Add the thing", "nothing typed is lost");
    assert!(pane.rows.last().unwrap().contains("tabs are not allowed"));
}

#[test]
fn nothing_staged_is_said_in_words() {
    let mut pane = pane_with(summary_of(1, 1, 0, None, 0, 0));
    pane.act("commit");
    pane.text("x");
    pane.key("enter");
    let request = pane.answer_request_of_commit();

    pane.hear(ToPlugin::Refused {
        request,
        reason: Refusal::NothingStaged,
    });

    assert!(pane.rows.last().unwrap().contains("stage a file first"));
}

#[test]
fn with_nothing_staged_a_commit_is_not_even_sent() {
    let mut pane = pane_with(summary_of(1, 0, 1, None, 0, 0));
    pane.act("commit");
    pane.text("x");

    pane.key("enter");

    assert!(pane.sent().is_empty());
    assert!(pane.rows.last().unwrap().contains("stage a file first"));
}

impl Pane {
    /// The request number of the commit it just sent.
    fn answer_request_of_commit(&self) -> u64 {
        self.said
            .iter()
            .find_map(|message| match message {
                FromPlugin::GitCommit { request, .. } => Some(*request),
                _ => None,
            })
            .expect("it sent a commit")
    }

    fn remote_request(&self) -> u64 {
        self.said
            .iter()
            .find_map(|message| match message {
                FromPlugin::GitFetch { request }
                | FromPlugin::GitPull { request }
                | FromPlugin::GitPush { request } => Some(*request),
                _ => None,
            })
            .expect("it sent a command to the remote")
    }
}

// --- fetch, pull, push ---

#[test]
fn each_button_sends_its_own_command_to_the_remote() {
    for (id, check) in [
        (
            "fetch",
            (|m: &FromPlugin| matches!(m, FromPlugin::GitFetch { .. })) as fn(&FromPlugin) -> bool,
        ),
        ("pull", |m| matches!(m, FromPlugin::GitPull { .. })),
        ("push", |m| matches!(m, FromPlugin::GitPush { .. })),
    ] {
        let mut pane = pane_with(summary_of(1, 0, 0, Some("origin/main"), 1, 1));
        pane.press(id);
        assert_eq!(pane.sent().len(), 1, "{id}");
        assert!(check(pane.sent()[0]), "{id}");
    }
}

#[test]
fn while_a_command_runs_it_is_named_the_others_wait_and_a_stop_appears() {
    let mut pane = pane_with(summary_of(1, 1, 1, Some("origin/main"), 1, 1));

    pane.press("push");

    assert!(pane.rows[0].contains("pushing…"), "{:?}", pane.rows);
    for id in ["commit", "fetch", "pull", "push"] {
        assert!(!pane.button(id).enabled, "{id}");
    }
    assert!(pane.has_button("cancel"));
    assert!(pane.button("cancel").enabled);
    // Staging goes on: it does not touch the remote.
    assert!(pane.button("stage_all").enabled);
}

#[test]
fn a_second_command_is_not_sent_while_one_runs() {
    let mut pane = pane_with(summary_of(1, 0, 0, Some("origin/main"), 1, 1));
    pane.press("push");

    pane.press("fetch");

    assert!(pane.sent().is_empty());
    assert!(
        pane.rows
            .last()
            .unwrap()
            .contains("another command is running")
    );
}

#[test]
fn when_the_command_is_done_the_buttons_come_back() {
    let mut pane = pane_with(summary_of(1, 0, 0, Some("origin/main"), 1, 1));
    pane.press("pull");
    let request = pane.remote_request();

    pane.hear(ToPlugin::Done { request });

    assert!(!pane.has_button("cancel"));
    assert!(pane.button("pull").enabled);
    assert!(!pane.rows[0].contains("pulling"));
}

#[test]
fn the_stop_asks_the_editor_to_stop_the_command() {
    let mut pane = pane_with(summary_of(1, 0, 0, Some("origin/main"), 1, 1));
    pane.press("push");

    pane.press("cancel");

    assert!(matches!(pane.sent()[..], [FromPlugin::GitCancel { .. }]));
    assert!(
        pane.rows[0].contains("pushing"),
        "it runs until the editor says it ended"
    );
}

#[test]
fn a_stop_with_nothing_running_does_nothing() {
    let mut pane = pane_with(summary_of(1, 0, 0, Some("origin/main"), 1, 1));
    pane.press("cancel");
    assert!(pane.sent().is_empty());
}

#[test]
fn a_command_that_was_stopped_or_failed_brings_the_buttons_back_and_says_so() {
    for (reason, said) in [
        (Refusal::Cancelled, "stopped"),
        (
            Refusal::Failed {
                detail: "Not possible to fast-forward".into(),
            },
            "Not possible to fast-forward",
        ),
        (
            Refusal::AuthRequired {
                detail: "terminal prompts disabled".into(),
            },
            "authenticate once in the terminal pane",
        ),
        (Refusal::Busy, "another command is running"),
        (
            Refusal::Denied {
                capability: trcode_plugin_api::Capability::GitRemote,
            },
            "not allowed: git.remote",
        ),
    ] {
        let mut pane = pane_with(summary_of(1, 0, 0, Some("origin/main"), 1, 1));
        pane.press("pull");
        let request = pane.remote_request();

        pane.hear(ToPlugin::Refused { request, reason });

        assert!(pane.button("pull").enabled, "{said}");
        assert!(!pane.has_button("cancel"), "{said}");
        assert!(
            pane.rows.last().unwrap().contains(said),
            "{said}: {:?}",
            pane.rows
        );
    }
}

#[test]
fn what_was_said_goes_away_with_the_next_thing_pressed() {
    let mut pane = pane_with(summary_of(1, 0, 0, Some("origin/main"), 1, 1));
    pane.press("pull");
    let request = pane.remote_request();
    pane.hear(ToPlugin::Refused {
        request,
        reason: Refusal::Cancelled,
    });
    assert!(pane.rows.last().unwrap().contains("stopped"));

    pane.press("fetch");

    assert!(!pane.rows.iter().any(|row| row == "stopped"));
}

#[test]
fn every_answer_still_ends_with_a_view_of_three_parts() {
    let mut pane = pane_with(summary_of(1, 1, 1, Some("origin/main"), 1, 1));
    for message in [
        ToPlugin::Refresh,
        ToPlugin::Press {
            button: "nonsense".into(),
        },
        ToPlugin::InputClick { column: 3 },
        ToPlugin::text("x"),
        ToPlugin::InputEnded,
        ToPlugin::GitUnavailable,
    ] {
        pane.hear(message);
        assert!(matches!(
            pane.said.last(),
            Some(FromPlugin::SetView {
                view: WireView::Column { .. }
            })
        ));
    }
}
