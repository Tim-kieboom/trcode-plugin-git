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

## Keys

| Key | Action |
| --- | --- |
| `up`, `down` | move over the files |
| `enter`, a click | open the file in the editor |
| `s` | stage the file (also an untracked one) |
| `u` | unstage the file |
| `r` | read the files again |
| `esc` | back to the editor |

They can be changed in your keymap (`context = "plugin"`, action
`{ plugin = "git:stage" }`, and so on); every action is in the command palette as `git: ...`.

## Development

The logic (`Viewer`) knows nothing of WebAssembly: `cargo test`. The built module, run by
the editor against a real repository, is tested from the editor's repository with
`TRCODE_PLUGIN_GIT_DIR=/path/to/this/repo cargo test -p terminal_code real_plugin -- --ignored`
(CI does it).

## Releasing

Set `version` in `manifest.toml`, tag `vX.Y.Z` and push the tag. The release workflow builds
the module, attaches it and the manifest to a release, and prints the entry to paste into
`plugins/index.json` of the editor's repository (in the run's summary).
