//! What the Git pane knows and what it shows.
//!
//! The editor tells it how the repository is (a summary: the branch and how many
//! files each group has) and the pane asks for the files it shows, a page at a
//! time, so a repository with thousands of changes costs only what is on screen.
//! The rows it sends are a window around the selected file, not all of them.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use trcode_plugin_api::{
    Capability, FromPlugin, GitEntry, GitKind, GitSection, GitSummary, Plugin, Refusal, ToPlugin,
    WireLine, WireSpan, WireView,
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
}

/// What a row of the last view was, for a click on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row {
    Branch,
    Header,
    File(usize),
    Other,
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
            ToPlugin::Done { request } => {
                self.requests.remove(&request);
            }
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
    }

    fn unavailable(&mut self) {
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

    fn refused(&mut self, request: u64, reason: &Refusal) {
        // A page that was refused stays marked as asked, so it is not asked for again
        // and again: the next summary (or a refresh) starts over.
        self.requests.remove(&request);
        self.problem = Some(match reason {
            Refusal::Denied { capability } => {
                format!("not allowed: {}", capability_name(*capability))
            }
            Refusal::NoRepository => "no git repository".to_owned(),
            Refusal::BadArgument { detail } | Refusal::Failed { detail } => detail.clone(),
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

    fn click(&mut self, row: usize, out: &mut Vec<FromPlugin>) {
        if let Some(Row::File(file)) = self.rows.get(row).copied() {
            self.selected = file;
            self.problem = None;
            self.open_selected(out);
        }
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

    /// What the pane shows now, and what each of its rows is.
    fn view(&mut self) -> WireView {
        let mut rows = Vec::new();
        let mut lines = Vec::new();
        let mut selected = None;
        match self.repository {
            None => lines.push(plain("…", HINT_SLOT)),
            Some(false) => lines.push(plain("No git repository", HINT_SLOT)),
            Some(true) => {
                lines.push(plain(&self.heading, BRANCH_SLOT));
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
                rows.push(Row::Header);
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
