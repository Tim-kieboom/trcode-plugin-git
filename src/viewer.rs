//! What the Git pane knows and what it shows.
//!
//! The editor tells it how the repository is (a summary: the branch and how many
//! files each group has) and the pane asks for the files it shows, a page at a
//! time, so a repository with thousands of changes costs only what is on screen.
//! The rows it sends are a window around the selected file, not all of them.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use trcode_plugin_api::{
    Capability, FromPlugin, GitEntry, GitKind, GitSection, GitSummary, Plugin, Refusal, ToPlugin,
    WireButton, WireLine, WireMenuItem, WireSpan, WireView,
};

/// How many files are asked for at once.
const PAGE: usize = 100;
/// How many files and headings are sent to the editor at once: the ones around the
/// selected file.
const WINDOW: usize = 80;

const HEADER_SLOT: &str = "explorer.dir";
const BRANCH_SLOT: &str = "explorer.active";
const NAME_SLOT: &str = "explorer.file";
const FOLDER_SLOT: &str = "explorer.ignored";
const HINT_SLOT: &str = "explorer.hint";
const ERROR_SLOT: &str = "notification.error";

/// The groups in the order they are shown.
const GROUPS: [(GitSection, &str); 4] = [
    (GitSection::Conflicts, "Merge Conflicts"),
    (GitSection::Staged, "Staged Changes"),
    (GitSection::Changes, "Changes"),
    (GitSection::Untracked, "Untracked"),
];

/// A group of files: how many there are and the pages of them that have come.
#[derive(Debug, Default)]
struct Group {
    count: usize,
    pages: BTreeMap<usize, Vec<GitEntry>>,
    /// The pages asked for and not here yet.
    asked: BTreeSet<usize>,
}

/// What a request that was sent was for.
#[derive(Debug, Clone, Copy)]
enum Asked {
    Page {
        group: usize,
        page: usize,
    },
    /// A change or an opening: nothing to do with the answer but tell what went wrong.
    Command,
    /// The commit: when it is done the message is cleared.
    Commit,
    /// A fetch, pull or push: when it ends the buttons come back.
    Remote,
}

/// The fetch, pull or push that is running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Running {
    request: u64,
    /// What it is called while it runs: `fetching`, `pulling`, `pushing`.
    doing: &'static str,
}

/// What the buttons need to know of the repository.
#[derive(Debug, Default, Clone)]
struct Facts {
    staged: usize,
    /// Files that are changed but not staged, untracked or in conflict.
    unstaged: usize,
    has_branch: bool,
    upstream: bool,
    ahead: u32,
    behind: u32,
}

/// Which command to the remote.
#[derive(Debug, Clone, Copy)]
enum Remote {
    Fetch,
    Pull,
    Push,
}

/// What a row of the last view was, for a click on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row {
    Branch,
    /// The heading of a group, by the place of the group in [`GROUPS`].
    Header(usize),
    File(usize),
    Other,
}

/// What the menu of a heading offers: take the whole group out of the staged changes, or stage
/// it. (Staging everything stages the files of the other groups too: the menu says so by being
/// "Stage All".)
fn group_menu(group: usize) -> Vec<WireMenuItem> {
    let all = match GROUPS[group].0 {
        GitSection::Staged => "unstage_all",
        _ => "stage_all",
    };
    vec![WireMenuItem::action(all)]
}

/// What the menu of the branch offers: the commands to the remote.
fn remote_menu() -> Vec<WireMenuItem> {
    vec![
        WireMenuItem::action("fetch"),
        WireMenuItem::action("pull"),
        WireMenuItem::action("push"),
        WireMenuItem::Separator,
        WireMenuItem::action("refresh"),
    ]
}

#[derive(Debug)]
pub struct Viewer {
    /// Whether the project is in a repository; not known before the editor says.
    repository: Option<bool>,
    generation: u64,
    heading: String,
    groups: [Group; 4],
    /// The selected file, counted over all the groups in the order they are shown.
    selected: usize,
    next_request: u64,
    requests: HashMap<u64, Asked>,
    /// What went wrong last, shown under the files until something else is done.
    problem: Option<String>,
    /// What the rows of the last view were.
    rows: Vec<Row>,
    facts: Facts,
    /// The commit message, and the character the cursor is before.
    message: String,
    cursor: usize,
    /// Whether the message is being typed into: the keys are its text.
    typing: bool,
    running: Option<Running>,
    /// Whether the list under the More button is open.
    more_open: bool,
}

impl Default for Viewer {
    fn default() -> Self {
        Self {
            repository: None,
            generation: 0,
            heading: String::new(),
            groups: Default::default(),
            selected: 0,
            next_request: 1,
            requests: HashMap::new(),
            problem: None,
            rows: Vec::new(),
            facts: Facts::default(),
            message: String::new(),
            cursor: 0,
            typing: false,
            running: None,
            more_open: false,
        }
    }
}

impl Plugin for Viewer {
    fn handle(&mut self, message: ToPlugin) -> Vec<FromPlugin> {
        let mut out = Vec::new();
        match message {
            ToPlugin::Start | ToPlugin::Refresh => {}
            ToPlugin::GitChanged { summary } => self.summary(&summary),
            ToPlugin::GitUnavailable => self.unavailable(),
            ToPlugin::GitEntries {
                request,
                generation,
                offset,
                entries,
                ..
            } => self.page(request, generation, offset, entries),
            ToPlugin::Action { action } => self.action(&action, &mut out),
            ToPlugin::Click { row } => self.click(row, &mut out),
            ToPlugin::ContextClick { row } => self.context_click(row, &mut out),
            ToPlugin::Press { button } => self.press(&button, &mut out),
            ToPlugin::InputClick { column } => self.click_in_message(column),
            ToPlugin::Text { text } => self.type_text(&text),
            ToPlugin::Key { key, ctrl, .. } => self.type_key(&key, ctrl, &mut out),
            ToPlugin::InputEnded => self.typing = false,
            ToPlugin::Done { request } => self.done(request),
            ToPlugin::Refused { request, reason } => self.refused(request, &reason),
            _ => {}
        }
        self.ask_for_missing_pages(&mut out);
        out.push(FromPlugin::set_view(self.view()));
        out
    }
}

impl Viewer {
    fn total(&self) -> usize {
        self.groups.iter().map(|group| group.count).sum()
    }

    fn summary(&mut self, summary: &GitSummary) {
        self.repository = Some(true);
        self.problem = None;
        self.generation = summary.generation;
        self.heading = heading(summary);
        let counts = [
            summary.conflicts,
            summary.staged,
            summary.changes,
            summary.untracked,
        ];
        for (group, count) in self.groups.iter_mut().zip(counts) {
            *group = Group {
                count,
                ..Group::default()
            };
        }
        // Pages that were asked for belong to the state before.
        self.requests
            .retain(|_, asked| !matches!(asked, Asked::Page { .. }));
        self.selected = self.selected.min(self.total().saturating_sub(1));
        self.facts = Facts {
            staged: summary.staged,
            unstaged: summary.changes + summary.untracked + summary.conflicts,
            has_branch: summary.branch.is_some(),
            upstream: summary.upstream.is_some(),
            ahead: summary.ahead,
            behind: summary.behind,
        };
    }

    fn unavailable(&mut self) {
        self.facts = Facts::default();
        self.repository = Some(false);
        self.groups = Default::default();
        self.requests.clear();
        self.selected = 0;
    }

    /// Takes a page of files that came, unless it is of a state that has gone.
    fn page(&mut self, request: u64, generation: u64, offset: usize, entries: Vec<GitEntry>) {
        let Some(Asked::Page { group, page }) = self.requests.remove(&request) else {
            return;
        };
        if generation != self.generation || offset != page * PAGE {
            // Of another state: a summary of it is on its way, and asks again.
            return;
        }
        self.groups[group].asked.remove(&page);
        self.groups[group].pages.insert(page, entries);
    }

    fn done(&mut self, request: u64) {
        match self.requests.remove(&request) {
            Some(Asked::Commit) => {
                self.message.clear();
                self.cursor = 0;
                self.typing = false;
            }
            Some(Asked::Remote) => self.running = None,
            _ => {}
        }
    }

    fn refused(&mut self, request: u64, reason: &Refusal) {
        // A page that was refused stays marked as asked, so it is not asked for again
        // and again: the next summary (or a refresh) starts over.
        let asked = self.requests.remove(&request);
        if matches!(asked, Some(Asked::Remote)) {
            self.running = None;
        }
        self.problem = Some(match reason {
            Refusal::Denied { capability } => {
                format!("not allowed: {}", capability_name(*capability))
            }
            Refusal::NoRepository => "no git repository".to_owned(),
            Refusal::BadArgument { detail } | Refusal::Failed { detail } => detail.clone(),
            Refusal::NothingStaged => "nothing is staged: stage a file first".to_owned(),
            Refusal::Busy => "another command is running".to_owned(),
            Refusal::Cancelled => "stopped".to_owned(),
            Refusal::AuthRequired { .. } => {
                "needs a password or key: authenticate once in the terminal pane".to_owned()
            }
            _ => "the editor said no".to_owned(),
        });
    }

    fn action(&mut self, action: &str, out: &mut Vec<FromPlugin>) {
        self.problem = None;
        match action {
            "up" => self.selected = self.selected.saturating_sub(1),
            "down" => self.selected = (self.selected + 1).min(self.total().saturating_sub(1)),
            "open" => self.open_selected(out),
            "stage" => self.change_selected(out, true),
            "unstage" => self.change_selected(out, false),
            "stage_all" => self.stage_all(out),
            "unstage_all" => self.unstage_all(out),
            "commit" => self.start_commit(),
            "more" => self.more_open = !self.more_open,
            "fetch" => self.remote(out, Remote::Fetch),
            "pull" => self.remote(out, Remote::Pull),
            "push" => self.remote(out, Remote::Push),
            "refresh" => {
                for group in &mut self.groups {
                    group.pages.clear();
                    group.asked.clear();
                }
                self.requests
                    .retain(|_, asked| !matches!(asked, Asked::Page { .. }));
            }
            _ => {}
        }
    }

    /// A button was pressed: the same as the action of that name, except that the
    /// commit button commits when there is a message and asks for one when not.
    fn press(&mut self, button: &str, out: &mut Vec<FromPlugin>) {
        self.problem = None;
        match button {
            "more" => self.more_open = !self.more_open,
            "commit" => self.commit_or_ask(out),
            "stage_all" => self.stage_all(out),
            "unstage_all" => self.unstage_all(out),
            "fetch" => self.remote(out, Remote::Fetch),
            "pull" => self.remote(out, Remote::Pull),
            "push" => self.remote(out, Remote::Push),
            "cancel" => self.cancel(out),
            _ => {}
        }
        // What the list offered is done: it closes.
        if matches!(button, "stage_all" | "unstage_all" | "fetch" | "pull") {
            self.more_open = false;
        }
    }

    fn stage_all(&mut self, out: &mut Vec<FromPlugin>) {
        if self.facts.unstaged == 0 {
            self.problem = Some("nothing to stage".to_owned());
            return;
        }
        let request = self.send(Asked::Command);
        out.push(FromPlugin::GitStageAll { request });
    }

    fn unstage_all(&mut self, out: &mut Vec<FromPlugin>) {
        if self.facts.staged == 0 {
            self.problem = Some("nothing is staged".to_owned());
            return;
        }
        let request = self.send(Asked::Command);
        out.push(FromPlugin::GitUnstageAll { request });
    }

    /// The `commit` action: moves to the message to type it.
    fn start_commit(&mut self) {
        self.typing = true;
        self.cursor = self.message.chars().count();
    }

    /// Commits with what was typed; with nothing typed it moves to the message.
    fn commit_or_ask(&mut self, out: &mut Vec<FromPlugin>) {
        if self.message.trim().is_empty() {
            self.problem = Some("type a commit message first".to_owned());
            self.start_commit();
            return;
        }
        if self.facts.staged == 0 {
            self.problem = Some("nothing is staged: stage a file first".to_owned());
            return;
        }
        let request = self.send(Asked::Commit);
        out.push(FromPlugin::GitCommit {
            request,
            message: self.message.clone(),
        });
    }

    fn remote(&mut self, out: &mut Vec<FromPlugin>, which: Remote) {
        if self.running.is_some() {
            self.problem = Some("another command is running".to_owned());
            return;
        }
        let request = self.send(Asked::Remote);
        let doing = match which {
            Remote::Fetch => {
                out.push(FromPlugin::GitFetch { request });
                "fetching"
            }
            Remote::Pull => {
                out.push(FromPlugin::GitPull { request });
                "pulling"
            }
            Remote::Push => {
                out.push(FromPlugin::GitPush { request });
                "pushing"
            }
        };
        self.running = Some(Running { request, doing });
    }

    fn cancel(&mut self, out: &mut Vec<FromPlugin>) {
        if self.running.is_none() {
            return;
        }
        let request = self.send(Asked::Command);
        out.push(FromPlugin::GitCancel { request });
    }

    /// A click in the message: it is typed into from the character clicked.
    fn click_in_message(&mut self, column: usize) {
        self.typing = true;
        self.cursor = column.min(self.message.chars().count());
    }

    fn type_text(&mut self, text: &str) {
        if !self.typing {
            return;
        }
        // One line: a pasted line break is a space.
        let text: String = text
            .chars()
            .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
            .collect();
        let at = self.byte_of(self.cursor);
        self.message.insert_str(at, &text);
        self.cursor += text.chars().count();
    }

    fn type_key(&mut self, key: &str, ctrl: bool, out: &mut Vec<FromPlugin>) {
        if !self.typing {
            return;
        }
        let length = self.message.chars().count();
        match (key, ctrl) {
            ("backspace", _) if self.cursor > 0 => {
                self.cursor -= 1;
                let at = self.byte_of(self.cursor);
                self.message.remove(at);
            }
            ("delete", _) if self.cursor < length => {
                let at = self.byte_of(self.cursor);
                self.message.remove(at);
            }
            ("left", _) => self.cursor = self.cursor.saturating_sub(1),
            ("right", _) => self.cursor = (self.cursor + 1).min(length),
            ("home", _) | ("a", true) => self.cursor = 0,
            ("end", _) | ("e", true) => self.cursor = length,
            // Clears the line.
            ("u", true) => {
                self.message.clear();
                self.cursor = 0;
            }
            ("enter", _) => {
                self.problem = None;
                self.commit_or_ask(out);
            }
            _ => {}
        }
    }

    /// Where character `index` starts in the message.
    fn byte_of(&self, index: usize) -> usize {
        self.message
            .char_indices()
            .nth(index)
            .map_or(self.message.len(), |(at, _)| at)
    }

    fn click(&mut self, row: usize, out: &mut Vec<FromPlugin>) {
        self.more_open = false;
        if let Some(Row::File(file)) = self.rows.get(row).copied() {
            self.selected = file;
            self.problem = None;
            self.open_selected(out);
        }
    }

    /// A right click on a row: a file is selected, and the menu is what can be done with it; a
    /// heading offers what can be done with the group, the branch what can be done with the
    /// remote. What is offered are actions of the manifest, so each has its title and its key.
    fn context_click(&mut self, row: usize, out: &mut Vec<FromPlugin>) {
        self.more_open = false;
        self.problem = None;
        let items = match self.rows.get(row).copied() {
            Some(Row::File(file)) => {
                self.selected = file;
                self.file_menu()
            }
            Some(Row::Header(group)) => group_menu(group),
            Some(Row::Branch) => remote_menu(),
            _ => vec![WireMenuItem::action("refresh")],
        };
        out.push(FromPlugin::Menu { items });
    }

    /// What the menu of the selected file offers: open it, and stage or unstage it, as the
    /// group it is in says. A file whose page has not come has nothing but a refresh.
    fn file_menu(&self) -> Vec<WireMenuItem> {
        let Some((group, entry)) = self.selection() else {
            return vec![WireMenuItem::action("refresh")];
        };
        let mut items = Vec::new();
        if entry.kind != GitKind::Deleted {
            items.push(WireMenuItem::action("open"));
            items.push(WireMenuItem::Separator);
        }
        items.push(WireMenuItem::action(match GROUPS[group].0 {
            GitSection::Staged => "unstage",
            _ => "stage",
        }));
        items
    }

    /// The selected file as the group it is in and the file, once it has come.
    fn selection(&self) -> Option<(usize, &GitEntry)> {
        self.file(self.selected)
    }

    fn file(&self, index: usize) -> Option<(usize, &GitEntry)> {
        let (group, row) = self.locate(index)?;
        let entry = self.groups[group]
            .pages
            .get(&(row / PAGE))?
            .get(row % PAGE)?;
        Some((group, entry))
    }

    /// The group a file is in and its place in the group.
    fn locate(&self, mut index: usize) -> Option<(usize, usize)> {
        for (group, shown) in self.groups.iter().enumerate() {
            if index < shown.count {
                return Some((group, index));
            }
            index -= shown.count;
        }
        None
    }

    fn open_selected(&mut self, out: &mut Vec<FromPlugin>) {
        let Some((_, entry)) = self.selection() else {
            return;
        };
        if entry.kind == GitKind::Deleted {
            self.problem = Some("nothing to open: the file is deleted".to_owned());
            return;
        }
        if entry.path.ends_with('/') {
            self.problem = Some("nothing to open: a folder of untracked files".to_owned());
            return;
        }
        let path = entry.path.clone();
        let request = self.send(Asked::Command);
        out.push(FromPlugin::OpenFile { request, path });
    }

    /// Stages (`stage`) or unstages the selected file, when it is in a group where
    /// that makes sense.
    fn change_selected(&mut self, out: &mut Vec<FromPlugin>, stage: bool) {
        let Some((group, entry)) = self.selection() else {
            return;
        };
        let section = GROUPS[group].0;
        let path = entry.path.clone();
        match (stage, section) {
            (true, GitSection::Changes | GitSection::Untracked | GitSection::Conflicts) => {
                let request = self.send(Asked::Command);
                out.push(FromPlugin::GitStage { request, path });
            }
            (false, GitSection::Staged) => {
                let request = self.send(Asked::Command);
                out.push(FromPlugin::GitUnstage { request, path });
            }
            (true, _) => self.problem = Some("already staged".to_owned()),
            (false, _) => self.problem = Some("not staged".to_owned()),
        }
    }

    fn send(&mut self, asked: Asked) -> u64 {
        let request = self.next_request;
        self.next_request += 1;
        self.requests.insert(request, asked);
        request
    }

    /// Asks for the pages of files the rows about to be shown need and that are not
    /// here or on their way.
    fn ask_for_missing_pages(&mut self, out: &mut Vec<FromPlugin>) {
        if self.repository != Some(true) {
            return;
        }
        let (first, last) = self.window();
        let mut wanted = Vec::new();
        let mut at = 0;
        for (group, shown) in self.groups.iter().enumerate() {
            if shown.count == 0 {
                continue;
            }
            // The heading takes the first place of the group, the files the rest.
            let files_from = at + 1;
            at = files_from + shown.count;
            let from = first.max(files_from);
            let to = last.min(at);
            if from >= to {
                continue;
            }
            let (row_from, row_to) = (from - files_from, to - files_from - 1);
            for page in row_from / PAGE..=row_to / PAGE {
                if !shown.pages.contains_key(&page) && !shown.asked.contains(&page) {
                    wanted.push((group, page));
                }
            }
        }
        for (group, page) in wanted {
            self.groups[group].asked.insert(page);
            let request = self.send(Asked::Page { group, page });
            out.push(FromPlugin::GitEntries {
                request,
                section: GROUPS[group].0,
                offset: page * PAGE,
                limit: PAGE,
            });
        }
    }

    /// The part of the list (headings and files, in order) that is sent: the
    /// places from `first` up to but not including `last`.
    fn window(&self) -> (usize, usize) {
        let length = self.list_length();
        let selected_at = self.list_place_of(self.selected).unwrap_or(0);
        let first = selected_at
            .saturating_sub(WINDOW / 2)
            .min(length.saturating_sub(WINDOW));
        (first, (first + WINDOW).min(length))
    }

    /// How many places the list has: a heading and the files for each group
    /// that has any.
    fn list_length(&self) -> usize {
        self.groups
            .iter()
            .filter(|group| group.count > 0)
            .map(|group| 1 + group.count)
            .sum()
    }

    /// The place in the list of the file `index`.
    fn list_place_of(&self, index: usize) -> Option<usize> {
        let (group, row) = self.locate(index)?;
        let before: usize = self.groups[..group]
            .iter()
            .filter(|shown| shown.count > 0)
            .map(|shown| 1 + shown.count)
            .sum();
        Some(before + 1 + row)
    }

    /// What the pane shows now: the buttons, the files, the message.
    fn view(&mut self) -> WireView {
        let rows = self.rows_view();
        let mut parts = vec![self.buttons()];
        if self.more_open {
            parts.push(self.more_buttons());
        }
        parts.push(rows);
        parts.push(self.message_line());
        WireView::column(parts)
    }

    /// The buttons: only what can be done now can be pressed. Commit and Push are the ones
    /// that are always there, with More, which opens the rest (fetch, pull, stage all and unstage
    /// all). While a command runs the others wait and the one that can be done is the stop.
    fn buttons(&self) -> WireView {
        let facts = &self.facts;
        let repository = self.repository == Some(true);
        let idle = repository && self.running.is_none();
        let count = |label: &str, n: u32| match n {
            0 => label.to_owned(),
            n => format!("{label} ({n})"),
        };
        let arrow = if self.more_open { "^" } else { "v" };
        let mut buttons = vec![
            WireButton::new("commit", "Commit").enabled(idle && facts.staged > 0),
            WireButton::new("push", count("Push", facts.ahead))
                .enabled(idle && facts.has_branch && (facts.ahead > 0 || !facts.upstream)),
            WireButton::new("more", format!("More {arrow}")).enabled(repository),
        ];
        if self.running.is_some() {
            buttons.push(WireButton::new("cancel", "Cancel"));
        }
        WireView::buttons(buttons)
    }

    /// What More opens: fetch and pull, which wait for a command that runs, and stage all and
    /// unstage all, which do not touch the remote.
    fn more_buttons(&self) -> WireView {
        let facts = &self.facts;
        let repository = self.repository == Some(true);
        let idle = repository && self.running.is_none();
        let count = |label: &str, n: u32| match n {
            0 => label.to_owned(),
            n => format!("{label} ({n})"),
        };
        WireView::buttons(vec![
            WireButton::new("fetch", "Fetch").enabled(idle),
            WireButton::new("pull", count("Pull", facts.behind)).enabled(idle && facts.upstream),
            WireButton::new("stage_all", "Stage All").enabled(repository && facts.unstaged > 0),
            WireButton::new("unstage_all", "Unstage All").enabled(repository && facts.staged > 0),
        ])
    }

    fn message_line(&self) -> WireView {
        match self.typing {
            true => WireView::input("Message: ", &self.message, self.cursor),
            false => WireView::idle_input("Message: ", &self.message, self.cursor),
        }
    }

    /// What the files show now, and what each of its rows is.
    fn rows_view(&mut self) -> WireView {
        let mut rows = Vec::new();
        let mut lines = Vec::new();
        let mut selected = None;
        match self.repository {
            None => lines.push(plain("…", HINT_SLOT)),
            Some(false) => lines.push(plain("No git repository", HINT_SLOT)),
            Some(true) => {
                let heading = match self.running {
                    Some(running) => format!("{} · {}…", self.heading, running.doing),
                    None => self.heading.clone(),
                };
                lines.push(plain(&heading, BRANCH_SLOT));
                rows.push(Row::Branch);
                if self.total() == 0 {
                    lines.push(plain("No changes", HINT_SLOT));
                    rows.push(Row::Other);
                } else {
                    self.list(&mut lines, &mut rows, &mut selected);
                }
            }
        }
        if let Some(problem) = &self.problem {
            lines.push(plain(problem, ERROR_SLOT));
            rows.push(Row::Other);
        }
        rows.resize(lines.len(), Row::Other);
        self.rows = rows;
        WireView::rows(lines, selected)
    }

    /// The rows of the window: headings and files.
    fn list(&self, lines: &mut Vec<WireLine>, rows: &mut Vec<Row>, selected: &mut Option<usize>) {
        let (first, last) = self.window();
        let mut place = 0;
        let mut file = 0;
        for (group, shown) in self.groups.iter().enumerate() {
            if shown.count == 0 {
                continue;
            }
            if (first..last).contains(&place) {
                let title = format!("{} ({})", GROUPS[group].1, shown.count);
                lines.push(plain(&title, HEADER_SLOT));
                rows.push(Row::Header(group));
            }
            place += 1;
            for row in 0..shown.count {
                if (first..last).contains(&place) {
                    if file == self.selected {
                        *selected = Some(lines.len());
                    }
                    lines.push(self.file_line(group, row));
                    rows.push(Row::File(file));
                }
                place += 1;
                file += 1;
            }
        }
    }

    fn file_line(&self, group: usize, row: usize) -> WireLine {
        let entry = self.groups[group]
            .pages
            .get(&(row / PAGE))
            .and_then(|page| page.get(row % PAGE));
        let Some(entry) = entry else {
            return plain("  …", HINT_SLOT);
        };
        let (letter, slot) = mark(entry.kind);
        let (folder, name) = split(&entry.path);
        let mut spans = vec![
            WireSpan::new(format!("{letter} "), slot),
            WireSpan::new(name, NAME_SLOT),
        ];
        let mut rest = String::new();
        if !folder.is_empty() {
            rest.push_str(&format!("  {folder}"));
        }
        if let Some(original) = &entry.original {
            rest.push_str(&format!("  ← {original}"));
        }
        if !rest.is_empty() {
            spans.push(WireSpan::new(rest, FOLDER_SLOT));
        }
        WireLine::of_spans(spans)
    }
}

fn plain(text: &str, slot: &str) -> WireLine {
    WireLine::new(text, slot)
}

/// The branch and how far it is from the branch it follows.
fn heading(summary: &GitSummary) -> String {
    let name = match (&summary.branch, &summary.commit) {
        (Some(branch), _) => branch.clone(),
        (None, Some(commit)) => format!("({})", commit.get(..7).unwrap_or(commit)),
        (None, None) => "(no commit)".to_owned(),
    };
    let mut heading = name;
    if summary.ahead > 0 {
        heading.push_str(&format!(" ↑{}", summary.ahead));
    }
    if summary.behind > 0 {
        heading.push_str(&format!(" ↓{}", summary.behind));
    }
    heading
}

/// The letter and the theme slot of a kind of change.
fn mark(kind: GitKind) -> (char, &'static str) {
    match kind {
        GitKind::Modified => ('M', "git.modified"),
        GitKind::TypeChanged => ('T', "git.modified"),
        GitKind::Added => ('A', "git.added"),
        GitKind::Copied => ('C', "git.added"),
        GitKind::Renamed => ('R', "git.added"),
        GitKind::Deleted => ('D', "git.deleted"),
        GitKind::Untracked => ('U', "git.untracked"),
        GitKind::Conflicted => ('!', "git.conflict"),
        _ => ('?', HINT_SLOT),
    }
}

/// A path as the folder it is in, with the `/` at its end, and the name; a
/// folder of untracked files keeps its `/`.
fn split(path: &str) -> (&str, &str) {
    let trimmed = path.strip_suffix('/').unwrap_or(path);
    match trimmed.rfind('/') {
        Some(at) => (&path[..=at], &path[at + 1..]),
        None => ("", path),
    }
}

fn capability_name(capability: Capability) -> &'static str {
    capability.name()
}

#[cfg(test)]
pub(crate) const PAGE_FOR_TESTS: usize = PAGE;
#[cfg(test)]
pub(crate) const WINDOW_FOR_TESTS: usize = WINDOW;
