# Git

The Git pane of TerminalCode: the branch on top, then the changed files in groups (merge conflicts, staged changes, changes, untracked files), each with a letter for what happened to it.

## What you can do

A row of buttons does the work, and each has a key:

- **Commit** (`c`): commits what is staged with the message under the buttons.
- **Push** (`shift+p`, with the number of commits you are ahead).
- **More** (`m`) opens a second row: **Fetch** (`f`), **Pull** (`p`, fast-forward only, so it never starts a merge, with the number of commits you are behind), **Stage All** (`a`) and **Unstage All** (`shift+a`). Choosing one does it and closes the row.
- A **right click** opens a menu like the editor's: on a file, open it and stage or unstage it; on a heading, stage or unstage the whole group; on the branch, fetch, pull and push.
- `s` and `u` stage and unstage the file you are on, `enter` opens it, `r` reads the files again, `esc` stops a fetch, pull or push that is running.

Fetch, pull and push never ask for a password: when the remote wants one the command fails and says so. Sign in once in the terminal pane, or set up a credential helper or an ssh agent.

## What it asks to be allowed to

Read the state of the git repository, change it (stage, unstage, commit), use the remote (fetch, pull, push: it uses the network) and open files.
