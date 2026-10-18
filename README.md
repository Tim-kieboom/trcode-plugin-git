# trcode-plugin-git

The Git pane of [TerminalCode](https://github.com/Tim-kieboom/TerminalCode), as a WebAssembly
plugin: the branch on top, then the changed files in groups (merge conflicts, staged
changes, changes, untracked files), each with a letter for what happened to it.

## Installing

In TerminalCode open "Plugins: Manage", choose Git and press `enter`: it is downloaded,
you are asked what it may do (read the git repository, change it, open files) and
where its view goes (a bar next to the Explorer or the Terminal, or a popup).

By hand: build it and put it in the plugins folder of the config folder
(`~/.config/terminalcode/plugins/git/` on Linux):

```sh
rustup target add wasm32-unknown-unknown
cargo build --target wasm32-unknown-unknown --release
dir=~/.config/terminalcode/plugins/git
mkdir -p "$dir"
cp manifest.toml "$dir/"
cp target/wasm32-unknown-unknown/release/trcode_plugin_git.wasm "$dir/git.wasm"
```

## What it shows

A row of buttons (they wrap onto more lines in a narrow pane), the branch and the
changed files in groups, and a line for the commit message:

| Button | Does | Key |
| --- | --- | --- |
| Commit | commits what is staged with the message under it; with no message it takes you to the line to type it | `c` types the message, `enter` in it commits |
| Stage All | stages every change, new files too | `a` |
| Unstage All | takes everything out of the staged changes | `shift+a` |
| Fetch | fetches from the remote | `f` |
| Pull (n) | `git pull --ff-only`: only moves forward, so it never starts a merge or a rebase; if the branches diverged it says so and changes nothing. `n` is how many commits you are behind | `p` |
| Push (n) | pushes the branch; a branch with no upstream is pushed with `-u` when the repository has one remote. `n` is how many commits you are ahead | `shift+p` |
| Cancel | stops the fetch, pull or push that is running (also `esc`) | `esc` |

Only what can be done now can be pressed: Commit needs something staged, Pull an
upstream, Push a commit to push (or a branch with no upstream yet). A fetch, pull or
push runs one at a time and the branch row says what it is doing (`main · pushing…`).

Fetch, pull and push never ask for a password: the editor owns the terminal, so
when the remote wants one the command fails at once and says to authenticate once in the
terminal pane (run the same git command there) or to set up a credential helper or an ssh
agent. Whatever git or a hook prints when something fails is shown by the editor until
you dismiss it.

## Keys

| Key | Action |
| --- | --- |
| `up`, `down` | move over the files |
| `enter`, a click | open the file in the editor |
| `s` | stage the file (also an untracked one) |
| `u` | unstage the file |
| `a`, `shift+a` | stage all, unstage all |
| `c`, a click on the message | type the commit message (`esc` stops typing and keeps it) |
| `f`, `p`, `shift+p` | fetch, pull, push |
| `r` | read the files again |
| `esc` | back to the editor (while a fetch, pull or push runs: stop it) |

They can be changed in your keymap (`context = "plugin"`, action
`{ plugin = "git:push" }`, and so on); every action is in the command palette as `git: ...`.

It asks to be allowed to read the state of the git repository, to change it (stage,
unstage, commit), to use the remote (fetch, pull, push: it uses the network) and to open
files; you are asked when you turn it on, and again when an update asks for more.

## Development

The logic (`Viewer`) knows nothing of WebAssembly: `cargo test`. It needs TerminalCode plugin API 1.4 (`abi = "1.4"` in the manifest), so an editor with an older plugin API does not offer it. The built module, run by
the editor against a real repository, is tested from the editor's repository with
`TRCODE_PLUGIN_GIT_DIR=/path/to/this/repo cargo test -p terminal_code real_plugin -- --ignored`
(CI does it).

## Releasing

Set `version` in `manifest.toml`, tag `vX.Y.Z` and push the tag. The release workflow builds
the module, attaches it and the manifest to a release, and prints the entry to paste into
`plugins/index.json` of the editor's repository (in the run's summary).
